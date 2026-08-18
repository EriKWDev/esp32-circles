//! Screens, transitions and hit testing.
//!
//! Layout lives in code rather than data, and the routine that draws a screen is
//! the same one that registers its touch targets - so a button can never drift
//! away from the region that activates it.
//!
//! Everything is placed inside a circle of radius SAFE_R about the screen
//! centre. The panel is addressed as a 480x480 square, but a 2.16" AMOLED of
//! this kind is round, and even on a square one a circular composition suits a
//! UI whose whole visual language is expanding discs. Nothing important is ever
//! put where a bezel might eat it.

use core::fmt::Write as _;

use crate::font::FontId;
use crate::gfx::{rgb, Align, Scene, H, W};
use crate::model::{Link, State};
use crate::touch::Event;

const CX: i32 = (W / 2) as i32;
const CY: i32 = (H / 2) as i32;
/// Keep-out radius: content beyond this risks the round panel's edge.
const SAFE_R: i32 = 226;

const INK: u16 = rgb(238, 245, 250);
const MUTED: u16 = rgb(122, 143, 158);
const DIM: u16 = rgb(70, 84, 96);

const BG_HOME: u16 = rgb(9, 13, 17);
const BG_INSPECT: u16 = rgb(8, 22, 48);
const BG_FORCE: u16 = rgb(38, 22, 4);
const BG_RUN: u16 = rgb(5, 30, 20);

const C_INSPECT: u16 = rgb(46, 104, 214);
const C_FORCE: u16 = rgb(226, 142, 24);
const C_RUN: u16 = rgb(30, 176, 108);
const C_CANCEL: u16 = rgb(212, 52, 48);

/// Fixed-capacity string, so labels can be formatted without an allocator.
struct Buf<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Buf<N> {
    fn new() -> Self {
        Self { bytes: [0; N], len: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            if self.len < N {
                self.bytes[self.len] = b;
                self.len += 1;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Inspect,
    Force,
    Running,
}

impl Screen {
    fn background(self) -> u16 {
        match self {
            Screen::Home => BG_HOME,
            Screen::Inspect => BG_INSPECT,
            Screen::Force => BG_FORCE,
            Screen::Running => BG_RUN,
        }
    }
    fn accent(self) -> u16 {
        match self {
            Screen::Home => C_RUN,
            Screen::Inspect => C_INSPECT,
            Screen::Force => C_FORCE,
            Screen::Running => C_RUN,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Back,
    Inspect,
    Force,
    Go,
    Cancel,
    Relay(usize),
    Slider,
}

#[derive(Clone, Copy)]
enum Zone {
    Disc { cx: i32, cy: i32, r: i32 },
    Rect { x0: i32, y0: i32, x1: i32, y1: i32 },
}

impl Zone {
    fn contains(&self, x: i32, y: i32) -> bool {
        match *self {
            // Generous by a few pixels: fingers are bigger than hit boxes.
            Zone::Disc { cx, cy, r } => {
                let dx = x - cx;
                let dy = y - cy;
                dx * dx + dy * dy <= (r + 6) * (r + 6)
            }
            Zone::Rect { x0, y0, x1, y1 } => {
                x >= x0 - 6 && x <= x1 + 6 && y >= y0 - 6 && y <= y1 + 6
            }
        }
    }
}

const MAX_ZONES: usize = 16;

/// What the UI wants the network layer to do.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Trigger { relay: u8, seconds: u32 },
    Stop,
}

/// A decorative expanding disc, spawned by every tap. This is the circles demo's
/// signature effect kept as the UI's tactile feedback.
#[derive(Clone, Copy)]
struct Ripple {
    x: i32,
    y: i32,
    born_ms: u32,
    max_r: i32,
    color: u16,
    active: bool,
}

const MAX_RIPPLES: usize = 6;
const RIPPLE_GROW_MS: u32 = 460;
const RIPPLE_FADE_MS: u32 = 260;

/// A screen change, animated as a disc of the destination's colour growing from
/// the point that was touched until it has swallowed the old screen.
#[derive(Clone, Copy)]
struct Wipe {
    to: Screen,
    x: i32,
    y: i32,
    born_ms: u32,
    color: u16,
}

const WIPE_COVER_MS: u32 = 400;
const WIPE_REVEAL_MS: u32 = 240;

pub struct Ui {
    pub screen: Screen,
    wipe: Option<Wipe>,
    ripples: [Ripple; MAX_RIPPLES],
    zones: [(Target, Zone); MAX_ZONES],
    n_zones: usize,
    /// Force menu: minutes 1..=10, and which usable relay is selected.
    pub minutes: u32,
    pub selected: usize,
    dragging_slider: bool,
    /// Set while a triggered run is expected but not yet confirmed by a poll, so
    /// the countdown can appear instantly instead of waiting a round-trip.
    pending_run_ms: Option<u32>,
}

const NO_RIPPLE: Ripple =
    Ripple { x: 0, y: 0, born_ms: 0, max_r: 0, color: 0, active: false };

impl Ui {
    pub const fn new() -> Self {
        Self {
            screen: Screen::Home,
            wipe: None,
            ripples: [NO_RIPPLE; MAX_RIPPLES],
            zones: [(Target::Back, Zone::Disc { cx: 0, cy: 0, r: 0 }); MAX_ZONES],
            n_zones: 0,
            minutes: 3,
            selected: 0,
            dragging_slider: false,
            pending_run_ms: None,
        }
    }

    fn zone(&mut self, target: Target, zone: Zone) {
        if self.n_zones < MAX_ZONES {
            self.zones[self.n_zones] = (target, zone);
            self.n_zones += 1;
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<Target> {
        self.zones[..self.n_zones]
            .iter()
            .find(|(_, z)| z.contains(x, y))
            .map(|(t, _)| *t)
    }

    fn ripple(&mut self, x: i32, y: i32, color: u16, now_ms: u32) {
        // Farthest screen corner, so the disc always has somewhere to grow to.
        let dx = x.max(W as i32 - x);
        let dy = y.max(H as i32 - y);
        let max_r = isqrt_i32(dx * dx + dy * dy).min(150);
        let slot = self
            .ripples
            .iter()
            .position(|r| !r.active)
            .unwrap_or(0);
        self.ripples[slot] = Ripple { x, y, born_ms: now_ms, max_r, color, active: true };
    }

    fn start_wipe(&mut self, to: Screen, x: i32, y: i32, now_ms: u32) {
        self.wipe = Some(Wipe { to, x, y, born_ms: now_ms, color: to.background() });
    }

    /// Feed one touch event. Returns the action the network layer should take.
    pub fn input(&mut self, ev: Event, state: &State, now_ms: u32) -> Action {
        // Ignore input while a wipe is covering the screen: the target that was
        // hit is already leaving, and letting a second tap through mid-animation
        // is how you end up two screens deep by accident.
        if self.wipe.is_some() {
            return Action::None;
        }

        match ev {
            Event::Press(x, y) => {
                if let Some(target) = self.hit(x, y) {
                    if target == Target::Slider {
                        self.dragging_slider = true;
                        self.minutes = minutes_from_y(y);
                        self.ripple(x, y, C_FORCE, now_ms);
                        return Action::None;
                    }
                    // Immediate feedback on press; the action itself fires on
                    // release, so a slide-off can still cancel it.
                    let color = match target {
                        Target::Inspect => C_INSPECT,
                        Target::Force => C_FORCE,
                        Target::Go => C_RUN,
                        Target::Cancel => C_CANCEL,
                        _ => self.screen.accent(),
                    };
                    self.ripple(x, y, color, now_ms);
                }
                Action::None
            }
            Event::Drag(x, y) => {
                if self.dragging_slider {
                    let _ = x;
                    self.minutes = minutes_from_y(y);
                }
                Action::None
            }
            Event::Release { x, y, tap } => {
                if self.dragging_slider {
                    self.dragging_slider = false;
                    return Action::None;
                }
                if !tap {
                    return Action::None;
                }
                let Some(target) = self.hit(x, y) else { return Action::None };
                match target {
                    Target::Back => {
                        let to = if self.screen == Screen::Running {
                            Screen::Home
                        } else {
                            Screen::Home
                        };
                        self.start_wipe(to, x, y, now_ms);
                        Action::None
                    }
                    Target::Inspect => {
                        self.start_wipe(Screen::Inspect, x, y, now_ms);
                        Action::None
                    }
                    Target::Force => {
                        self.selected = self.selected.min(state.n_usable().saturating_sub(1));
                        self.start_wipe(Screen::Force, x, y, now_ms);
                        Action::None
                    }
                    Target::Relay(index) => {
                        self.selected = index;
                        Action::None
                    }
                    Target::Go => {
                        let relay = state
                            .usable()
                            .nth(self.selected)
                            .map(|r| r.id)
                            .unwrap_or(0);
                        if relay == 0 {
                            return Action::None;
                        }
                        self.pending_run_ms = Some(now_ms);
                        self.start_wipe(Screen::Running, x, y, now_ms);
                        Action::Trigger { relay, seconds: self.minutes * 60 }
                    }
                    Target::Cancel => {
                        self.pending_run_ms = None;
                        // Red disc from the cancel button, then home.
                        self.wipe = Some(Wipe {
                            to: Screen::Home,
                            x,
                            y,
                            born_ms: now_ms,
                            color: C_CANCEL,
                        });
                        Action::Stop
                    }
                    Target::Slider => Action::None,
                }
            }
            Event::None => Action::None,
        }
    }

    /// Advance animations, and follow the controller into/out of a run.
    pub fn update(&mut self, state: &State, now_ms: u32) {
        if let Some(w) = self.wipe {
            if now_ms.wrapping_sub(w.born_ms) >= WIPE_COVER_MS + WIPE_REVEAL_MS {
                self.screen = w.to;
                self.wipe = None;
            }
        }
        for r in self.ripples.iter_mut() {
            if r.active && now_ms.wrapping_sub(r.born_ms) >= RIPPLE_GROW_MS + RIPPLE_FADE_MS {
                r.active = false;
            }
        }

        // A run starting or stopping is the controller's decision - it may come
        // from the schedule or another client, not just from this panel - so the
        // screen follows the reported state rather than only local taps.
        if self.wipe.is_none() {
            if state.running && self.screen != Screen::Running && self.screen != Screen::Force {
                self.start_wipe(Screen::Running, CX, CY, now_ms);
            }
            if !state.running && self.screen == Screen::Running {
                let stale = self
                    .pending_run_ms
                    .map_or(true, |t| now_ms.wrapping_sub(t) > 4_000);
                if stale {
                    self.pending_run_ms = None;
                    self.start_wipe(Screen::Home, CX, CY, now_ms);
                }
            }
        }
        if state.running {
            self.pending_run_ms = None;
        }
    }

    pub fn animating(&self) -> bool {
        self.wipe.is_some() || self.ripples.iter().any(|r| r.active)
    }

    /// Build the frame. Registers hit zones for whichever screen is current.
    pub fn build(&mut self, scene: &mut Scene, state: &State, now_ms: u32) {
        self.n_zones = 0;

        let (base, incoming) = match self.wipe {
            None => (self.screen, None),
            Some(w) => (self.screen, Some(w)),
        };

        scene.clear(base.background());

        // The screen being left is still drawn underneath the growing disc, so
        // the transition reads as one surface covering another rather than a cut.
        let content_alpha = match incoming {
            None => 255,
            Some(w) => {
                let age = now_ms.wrapping_sub(w.born_ms);
                if age >= WIPE_COVER_MS {
                    0
                } else {
                    255
                }
            }
        };
        if content_alpha > 0 {
            self.draw_screen(scene, base, state, now_ms, 255, incoming.is_none());
        }

        if let Some(w) = incoming {
            let age = now_ms.wrapping_sub(w.born_ms);
            if age < WIPE_COVER_MS {
                let t = (age * 32_768 / WIPE_COVER_MS).min(32_768);
                let eased = ease_out_q15(t);
                let dx = w.x.max(W as i32 - w.x);
                let dy = w.y.max(H as i32 - w.y);
                let max_r = isqrt_i32(dx * dx + dy * dy) + 4;
                let r = (max_r as u32 * eased / 32_768) as i32;
                scene.disc(w.x, w.y, r, w.color, 255);
            } else {
                // Covered: the destination owns the screen, and its content
                // fades up. Registers the destination's zones so it is
                // interactive the moment it is legible.
                scene.clear(w.color);
                let reveal = (age - WIPE_COVER_MS).min(WIPE_REVEAL_MS);
                let t = (reveal * 32_768 / WIPE_REVEAL_MS).min(32_768);
                let alpha = (smoothstep_q15(t) * 255 / 32_768) as u8;
                self.draw_screen(scene, w.to, state, now_ms, alpha, true);
            }
        }

        // Ripples ride on top of everything: they are feedback, not content.
        for i in 0..MAX_RIPPLES {
            let r = self.ripples[i];
            if !r.active {
                continue;
            }
            let age = now_ms.wrapping_sub(r.born_ms);
            let (radius, alpha) = if age < RIPPLE_GROW_MS {
                let t = age * 32_768 / RIPPLE_GROW_MS;
                let eased = ease_out_q15(t);
                ((r.max_r as u32 * eased / 32_768) as i32, 150u32)
            } else {
                let t = (age - RIPPLE_GROW_MS) * 32_768 / RIPPLE_FADE_MS;
                let fade = 32_768 - smoothstep_q15(t.min(32_768));
                (r.max_r, 150 * fade / 32_768)
            };
            // Drawn as a soft expanding ring so it reads as a ripple and never
            // hides the label underneath it.
            let thickness = (radius / 7).clamp(3, 16);
            scene.ring(
                r.x,
                r.y,
                radius,
                (radius - thickness).max(0),
                r.color,
                alpha as u8,
            );
        }
    }

    fn draw_screen(
        &mut self,
        scene: &mut Scene,
        screen: Screen,
        state: &State,
        now_ms: u32,
        alpha: u8,
        register: bool,
    ) {
        match screen {
            Screen::Home => self.draw_home(scene, state, alpha, register),
            Screen::Inspect => self.draw_inspect(scene, state, alpha, register),
            Screen::Force => self.draw_force(scene, state, alpha, register),
            Screen::Running => self.draw_running(scene, state, now_ms, alpha, register),
        }
    }

    fn draw_link(&self, scene: &mut Scene, state: &State, alpha: u8) {
        // A single dot: green online, amber connecting, red offline. Small on
        // purpose - it matters only when it is wrong.
        let color = match state.link {
            Link::Online => C_RUN,
            Link::Connecting => C_FORCE,
            Link::Offline => C_CANCEL,
        };
        scene.disc(CX, 42, 7, color, alpha);
    }

    fn draw_back(&mut self, scene: &mut Scene, alpha: u8, register: bool) {
        let (bx, by, br) = (108, 106, 36);
        scene.ring(bx, by, br, br - 4, MUTED, alpha);
        // Chevron from two short pills; the panel cannot rotate a primitive, so
        // the arrow is drawn as a pair of stacked steps that read as one at this
        // size.
        scene.pill(bx - 11, by - 3, bx + 9, by + 3, 3, INK, alpha);
        scene.pill(bx - 11, by - 11, bx - 5, by + 11, 3, INK, alpha);
        if register {
            self.zone(Target::Back, Zone::Disc { cx: bx, cy: by, r: br });
        }
    }

    fn draw_home(&mut self, scene: &mut Scene, state: &State, alpha: u8, register: bool) {
        self.draw_link(scene, state, alpha);

        // Clock.
        let mut clock = Buf::<8>::new();
        if state.clock_valid {
            let _ = write!(clock, "{:02}:{:02}", state.hh, state.mm);
        } else {
            let _ = write!(clock, "--:--");
        }
        scene.label(CX, 152, FontId::Display, INK, alpha, Align::Center, clock.as_str());

        // Next scheduled watering.
        let mut next = Buf::<40>::new();
        match state.next_start() {
            Some((s, minutes)) => {
                let name = state
                    .relay_by_id(s.entries[0].relay as i32)
                    .map(|r| r.name)
                    .unwrap_or(crate::gfx::Text::EMPTY);
                if minutes < 60 {
                    let _ = write!(next, "NEXT {:02}:{:02} IN {} MIN", s.hh, s.mm, minutes);
                } else {
                    let _ = write!(
                        next,
                        "NEXT {:02}:{:02} \u{b7} {}",
                        s.hh,
                        s.mm,
                        name.as_str()
                    );
                }
            }
            None => {
                let _ = write!(next, "NO SCHEDULE ARMED");
            }
        }
        scene.label(CX, 200, FontId::Caption, MUTED, alpha, Align::Center, next.as_str());

        // Inspect.
        let (ix0, iy0, ix1, iy1) = (78, 228, 402, 300);
        scene.pill(ix0, iy0, ix1, iy1, 36, C_INSPECT, alpha);
        scene.label(CX, 277, FontId::Body, INK, alpha, Align::Center, "INSPECT");
        if register {
            self.zone(Target::Inspect, Zone::Rect { x0: ix0, y0: iy0, x1: ix1, y1: iy1 });
        }

        // Force - the hero action, so it is the disc.
        scene.disc(CX, 380, 86, C_FORCE, alpha);
        scene.label(CX, 396, FontId::Body, rgb(24, 14, 2), alpha, Align::Center, "FORCE");
        if register {
            self.zone(Target::Force, Zone::Disc { cx: CX, cy: 380, r: 86 });
        }
    }

    fn draw_inspect(&mut self, scene: &mut Scene, state: &State, alpha: u8, register: bool) {
        self.draw_back(scene, alpha, register);
        scene.label(CX, 112, FontId::Caption, MUTED, alpha, Align::Center, "SCHEDULE");

        let mut row_y = 186;
        for index in 0..state.n_starts {
            let s = state.starts[index];
            let on = s.enabled && s.n_entries > 0;
            let (x0, x1) = (66, 414);
            let (y0, y1) = (row_y - 33, row_y + 33);
            scene.pill(x0, y0, x1, y1, 24, if on { rgb(16, 40, 84) } else { rgb(12, 20, 34) }, alpha);
            // Enabled marker.
            scene.disc(x0 + 30, row_y, 9, if on { C_RUN } else { DIM }, alpha);

            let mut time = Buf::<8>::new();
            let _ = write!(time, "{:02}:{:02}", s.hh, s.mm);
            scene.label(
                x0 + 54,
                row_y + 13,
                FontId::Body,
                if on { INK } else { DIM },
                alpha,
                Align::Left,
                time.as_str(),
            );

            let mut summary = Buf::<28>::new();
            if s.n_entries == 0 {
                let _ = write!(summary, "EMPTY");
            } else {
                let total = s.total_seconds();
                if total >= 60 {
                    let _ = write!(summary, "{} ZONES \u{b7} {} MIN", s.n_entries, total / 60);
                } else {
                    let _ = write!(summary, "{} ZONES \u{b7} {} S", s.n_entries, total);
                }
            }
            scene.label(
                x1 - 26,
                row_y + 10,
                FontId::Caption,
                if on { MUTED } else { DIM },
                alpha,
                Align::Right,
                summary.as_str(),
            );
            row_y += 78;
        }

        // Sensor line: the controller exposes several analog inputs; show the
        // first as a liveness cue rather than pretending to interpret it.
        if state.n_analogs > 0 {
            let a = state.analogs[0];
            let mut line = Buf::<28>::new();
            let _ = write!(line, "{} {} / 4095", a.name.as_str(), a.level);
            scene.label(CX, 432, FontId::Caption, DIM, alpha, Align::Center, line.as_str());
        }
    }

    fn draw_force(&mut self, scene: &mut Scene, state: &State, alpha: u8, register: bool) {
        self.draw_back(scene, alpha, register);

        // Duration readout.
        let mut mins = Buf::<4>::new();
        let _ = write!(mins, "{}", self.minutes);
        scene.label(246, 158, FontId::Display, INK, alpha, Align::Right, mins.as_str());
        scene.label(258, 158, FontId::Caption, MUTED, alpha, Align::Left, "MIN");

        // Slider: track, filled portion, knob.
        let (sx, top, bottom) = (SLIDER_X, SLIDER_TOP, SLIDER_BOTTOM);
        scene.pill(sx - 25, top, sx + 25, bottom, 25, rgb(58, 36, 8), alpha);
        let knob_y = y_from_minutes(self.minutes);
        scene.pill(sx - 25, knob_y, sx + 25, bottom, 25, C_FORCE, alpha);
        scene.disc(sx, knob_y, 28, rgb(252, 214, 150), alpha);
        if register {
            self.zone(
                Target::Slider,
                Zone::Rect { x0: sx - 34, y0: top - 20, x1: sx + 34, y1: bottom + 20 },
            );
        }

        // Relay chooser.
        let mut row_y = 206;
        for (index, relay) in state.usable().enumerate().take(5) {
            let selected = index == self.selected;
            let (x0, x1) = (150, 344);
            let (y0, y1) = (row_y - 22, row_y + 22);
            scene.pill(
                x0,
                y0,
                x1,
                y1,
                22,
                if selected { C_FORCE } else { rgb(52, 34, 10) },
                alpha,
            );
            scene.label(
                (x0 + x1) / 2,
                row_y + 9,
                FontId::Caption,
                if selected { rgb(26, 14, 0) } else { MUTED },
                alpha,
                Align::Center,
                relay.name.as_str(),
            );
            if register {
                self.zone(Target::Relay(index), Zone::Rect { x0, y0, x1, y1 });
            }
            row_y += 48;
        }

        // Go.
        scene.disc(392, 302, 56, C_RUN, alpha);
        scene.label(392, 318, FontId::Body, rgb(2, 22, 12), alpha, Align::Center, "GO!");
        if register {
            self.zone(Target::Go, Zone::Disc { cx: 392, cy: 302, r: 56 });
        }
    }

    fn draw_running(
        &mut self,
        scene: &mut Scene,
        state: &State,
        now_ms: u32,
        alpha: u8,
        register: bool,
    ) {
        let _ = now_ms;
        self.draw_back(scene, alpha, register);

        // Progress ring: full circumference as the track, swept portion as the
        // time already spent.
        scene.ring(CX, CY, 224, 212, rgb(10, 52, 36), alpha);
        let total = self.minutes * 60;
        let left = state.left_s.min(total.max(1));
        let done = total.saturating_sub(left);
        if total > 0 && done > 0 {
            let sweep = (done * 4096 / total).min(4095) as i32;
            scene.arc(CX, CY, 224, 212, 0, sweep, C_RUN, alpha);
        }

        // Which relay.
        let name = state
            .relay_by_id(state.active)
            .map(|r| r.name)
            .unwrap_or(crate::gfx::Text::EMPTY);
        scene.label(
            CX,
            150,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            if name.len > 0 { name.as_str() } else { "WATERING" },
        );

        // The countdown itself - the reason this screen exists, so it gets the
        // largest type on the device.
        let mut big = Buf::<10>::new();
        let _ = write!(big, "{}:{:02}", state.left_s / 60, state.left_s % 60);
        scene.label(CX, 326, FontId::Countdown, INK, alpha, Align::Center, big.as_str());

        if state.queued > 0 {
            let mut q = Buf::<24>::new();
            let _ = write!(q, "{} MORE QUEUED", state.queued);
            scene.label(CX, 362, FontId::Caption, MUTED, alpha, Align::Center, q.as_str());
        }

        // Cancel.
        let (x0, y0, x1, y1) = (152, 386, 328, 442);
        scene.pill(x0, y0, x1, y1, 28, C_CANCEL, alpha);
        scene.label(CX, 424, FontId::Body, INK, alpha, Align::Center, "CANCEL");
        if register {
            self.zone(Target::Cancel, Zone::Rect { x0, y0, x1, y1 });
        }
    }
}

const SLIDER_X: i32 = 88;
const SLIDER_TOP: i32 = 176;
const SLIDER_BOTTOM: i32 = 376;

/// Slider maps top = 10 minutes, bottom = 1 minute.
fn minutes_from_y(y: i32) -> u32 {
    let span = SLIDER_BOTTOM - SLIDER_TOP;
    let clamped = y.clamp(SLIDER_TOP, SLIDER_BOTTOM);
    let from_bottom = SLIDER_BOTTOM - clamped;
    // +span/18 biases rounding so each of the ten steps owns an equal slice.
    (1 + (from_bottom * 9 + span / 2) / span).clamp(1, 10) as u32
}

fn y_from_minutes(minutes: u32) -> i32 {
    let span = SLIDER_BOTTOM - SLIDER_TOP;
    SLIDER_BOTTOM - ((minutes as i32 - 1) * span) / 9
}

#[inline]
fn smoothstep_q15(t: u32) -> u32 {
    let t = t.min(32_768);
    let squared = t * t >> 15;
    squared * (3 * 32_768 - 2 * t) >> 15
}

/// 1-(1-t)^3: fast off the mark, settles gently. Used for anything the finger
/// just launched, because it makes the response feel immediate.
#[inline]
fn ease_out_q15(t: u32) -> u32 {
    let t = t.min(32_768);
    let inv = 32_768 - t;
    let cube = ((inv * inv >> 15) * inv) >> 15;
    32_768 - cube
}

#[inline]
fn isqrt_i32(n: i32) -> i32 {
    let mut n = n as u32;
    let mut res = 0u32;
    let mut bit = 1u32 << 30;
    while bit > n {
        bit >>= 2;
    }
    while bit != 0 {
        if n >= res + bit {
            n -= res + bit;
            res = (res >> 1) + bit;
        } else {
            res >>= 1;
        }
        bit >>= 2;
    }
    res as i32
}
