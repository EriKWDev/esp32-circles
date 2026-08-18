//! Screens, transitions and hit testing.
//!
//! Layout geometry lives in the `L` constants below and is used by *both* the
//! drawing code and the single zone-registration routine, so a button can never
//! drift away from the region that activates it.
//!
//! Everything is placed inside a circle of radius SAFE_R about the screen
//! centre. The panel is addressed as a 480x480 square, but a 2.16" AMOLED of
//! this kind is round, and even on a square one a circular composition suits a
//! UI whose whole visual language is expanding discs. Nothing important is put
//! where a bezel might eat it.

use core::fmt::Write as _;

use crate::font::FontId;
use crate::gfx::{rgb, Align, Scene, Text, H, W};
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

/// Layout. Shared by drawing and hit testing - see the module note.
mod l {
    /// (x0, y0, x1, y1) for the two home buttons. Identical size and corner
    /// radius: they are peers, so they are differentiated by colour and position
    /// rather than by shape, and the generous gap keeps them from touching.
    pub const HOME_INSPECT: (i32, i32, i32, i32) = (84, 236, 396, 308);
    pub const HOME_FORCE: (i32, i32, i32, i32) = (84, 330, 396, 402);
    pub const HOME_PILL_R: i32 = 36;

    /// (cx, cy, r). The panel is square with rounded corners, not round, so this
    /// sits properly in the top-left instead of being pulled toward the middle.
    pub const BACK: (i32, i32, i32) = (58, 58, 40);
    /// Clear of the relay list, which ends at RELAY_X1.
    pub const GO: (i32, i32, i32) = (406, 302, 58);
    /// (x0, y0, x1, y1) - centred under the countdown digits and comfortably
    /// inside RING_INNER, so it never crosses the ring.
    pub const CANCEL: (i32, i32, i32, i32) = (150, 356, 330, 418);

    /// Progress ring: a closed circle, swept clockwise from 12 o'clock.
    pub const RING_OUTER: i32 = 232;
    pub const RING_INNER: i32 = 218;

    pub const SLIDER_X: i32 = 88;
    pub const SLIDER_TOP: i32 = 176;
    pub const SLIDER_BOTTOM: i32 = 376;
    pub const SLIDER_HALF_W: i32 = 25;

    pub const RELAY_X0: i32 = 150;
    pub const RELAY_X1: i32 = 344;
    pub const RELAY_FIRST_CY: i32 = 206;
    pub const RELAY_PITCH: i32 = 48;
    pub const RELAY_HALF_H: i32 = 22;
    pub const RELAY_MAX_ROWS: usize = 5;

    /// Schedule rows use nearly the full panel width. They were inset by 66 px a
    /// side, which squeezed the clock and the zone summary into each other for no
    /// reason - the panel is square, so that margin was pure waste.
    pub const SCHED_X0: i32 = 26;
    pub const SCHED_X1: i32 = 454;
    pub const SCHED_FIRST_CY: i32 = 186;
    pub const SCHED_PITCH: i32 = 76;
    pub const SCHED_HALF_H: i32 = 30;
    pub const SCHED_MAX_ROWS: usize = 4;

    pub const DETAIL_FIRST_CY: i32 = 200;
    pub const DETAIL_PITCH: i32 = 48;
    pub const DETAIL_MAX_ROWS: usize = 5;
}

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
    /// One schedule's running order, opened from the schedules list.
    Detail,
}

impl Screen {
    fn background(self) -> u16 {
        match self {
            Screen::Home => BG_HOME,
            Screen::Inspect => BG_INSPECT,
            Screen::Force => BG_FORCE,
            Screen::Running => BG_RUN,
            Screen::Detail => BG_INSPECT,
        }
    }
    fn accent(self) -> u16 {
        match self {
            Screen::Home => C_RUN,
            Screen::Inspect => C_INSPECT,
            Screen::Force => C_FORCE,
            Screen::Running => C_RUN,
            Screen::Detail => C_INSPECT,
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
    Schedule(usize),
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

/// A decorative expanding ring, spawned by every tap. The circles demo's
/// signature effect, kept as the UI's tactile feedback.
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
/// the point touched until it has swallowed the old screen.
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
/// Belt and braces: no transition may ever outlive this, whatever the clock
/// does. A stuck wipe would swallow all input, which is the worst failure this
/// UI can have.
const WIPE_MAX_MS: u32 = 2_000;

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
    /// When the current slider drag began. A drag is abandoned after
    /// DRAG_MAX_MS so a touch controller that latches a contact - and therefore
    /// never reports a release - cannot strand the UI in drag mode.
    drag_started_ms: u32,
    /// Which schedule the detail screen is showing.
    pub detail: usize,
    /// The knob's *displayed* position, eased toward the selected minute. Values
    /// that snap look mechanical, so the knob chases its target - the same reason
    /// the progress sweep is driven from fractional time. Q4 for smooth easing at
    /// sub-pixel steps.
    knob_q4: i32,
    /// Previous `state.running`, so entering and leaving the countdown is driven
    /// by the *edges* of that flag.
    ///
    /// This was level-triggered once, and it deadlocked the UI: tapping Back on
    /// the countdown moved to Home, Home immediately saw `running` still true and
    /// wiped straight back, and because input is ignored while a wipe is in
    /// flight the panel ping-ponged with nothing responding. Edges also express
    /// the real intent - follow the controller when a run *starts* or *ends*, and
    /// otherwise leave navigation to whoever is holding the panel.
    last_running: bool,
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
            drag_started_ms: 0,
            detail: 0,
            knob_q4: 0,
            last_running: false,
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
        let dx = x.max(W as i32 - x);
        let dy = y.max(H as i32 - y);
        let max_r = isqrt_i32(dx * dx + dy * dy).min(150);
        let slot = self.ripples.iter().position(|r| !r.active).unwrap_or(0);
        self.ripples[slot] = Ripple { x, y, born_ms: now_ms, max_r, color, active: true };
    }

    fn start_wipe(&mut self, to: Screen, x: i32, y: i32, now_ms: u32) {
        self.wipe = Some(Wipe { to, x, y, born_ms: now_ms, color: to.background() });
    }

    /// Which screen a touch belongs to: once a wipe starts, the destination
    /// already owns input, even while it is still being covered.
    fn interactive_screen(&self) -> Screen {
        self.wipe.map_or(self.screen, |w| w.to)
    }

    /// Feed one touch event. Returns the action the network layer should take.
    ///
    /// Buttons act on PRESS, not release. Release is only knowable after
    /// RELEASE_MS of silence from the touch controller, so acting on it charged
    /// every tap a fixed latency before anything happened at all - which is most
    /// of why this felt sluggish. It was also a liveness hazard: if the
    /// controller ever latched a contact, the release never arrived and the UI
    /// stopped accepting input entirely. Acting on press removes both, at the
    /// cost of not being able to slide off a button to cancel it - a trade worth
    /// making for a panel whose buttons are this large.
    pub fn input(&mut self, ev: Event, state: &State, now_ms: u32) -> Action {
        // Ignore input while a transition runs: the target that was hit is
        // already leaving, and letting a second tap through mid-animation is how
        // you end up two screens deep by accident.
        if self.wipe.is_some() {
            return Action::None;
        }

        match ev {
            Event::Press(x, y) => {
                let Some(target) = self.hit(x, y) else { return Action::None };
                if target == Target::Slider {
                    self.dragging_slider = true;
                    self.drag_started_ms = now_ms;
                    self.minutes = minutes_from_y(y);
                    return Action::None;
                }
                let color = match target {
                    Target::Inspect => C_INSPECT,
                    Target::Force => C_FORCE,
                    Target::Go => C_RUN,
                    Target::Cancel => C_CANCEL,
                    _ => self.screen.accent(),
                };
                self.ripple(x, y, color, now_ms);

                match target {
                    Target::Back => {
                        // Detail belongs to the schedules list, so Back steps up
                        // one level there rather than jumping straight home.
                        let to = if self.screen == Screen::Detail {
                            Screen::Inspect
                        } else {
                            Screen::Home
                        };
                        self.start_wipe(to, x, y, now_ms);
                        Action::None
                    }
                    Target::Schedule(index) => {
                        self.detail = index;
                        self.start_wipe(Screen::Detail, x, y, now_ms);
                        Action::None
                    }
                    Target::Inspect => {
                        self.start_wipe(Screen::Inspect, x, y, now_ms);
                        Action::None
                    }
                    Target::Force => {
                        self.selected =
                            self.selected.min(state.n_usable().saturating_sub(1));
                        self.start_wipe(Screen::Force, x, y, now_ms);
                        Action::None
                    }
                    Target::Relay(index) => {
                        self.selected = index;
                        Action::None
                    }
                    Target::Go => {
                        let relay =
                            state.usable().nth(self.selected).map(|r| r.id).unwrap_or(0);
                        if relay == 0 {
                            return Action::None;
                        }
                        self.start_wipe(Screen::Running, x, y, now_ms);
                        Action::Trigger { relay, seconds: self.minutes * 60 }
                    }
                    Target::Cancel => {
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
            Event::Drag(_, y) => {
                if self.dragging_slider {
                    self.minutes = minutes_from_y(y);
                }
                Action::None
            }
            Event::Release { .. } => {
                self.dragging_slider = false;
                Action::None
            }
            Event::None => Action::None,
        }
    }

    /// Advance animations, and follow the controller into and out of a run.
    pub fn update(&mut self, state: &State, now_ms: u32) {
        if let Some(w) = self.wipe {
            let age = now_ms.wrapping_sub(w.born_ms);
            if age >= WIPE_COVER_MS + WIPE_REVEAL_MS || age > WIPE_MAX_MS {
                self.screen = w.to;
                self.wipe = None;
            }
        }
        for r in self.ripples.iter_mut() {
            if r.active && now_ms.wrapping_sub(r.born_ms) >= RIPPLE_GROW_MS + RIPPLE_FADE_MS {
                r.active = false;
            }
        }
        // A drag cannot outlive this. See `drag_started_ms`.
        const DRAG_MAX_MS: u32 = 8_000;
        if self.dragging_slider && now_ms.wrapping_sub(self.drag_started_ms) > DRAG_MAX_MS {
            self.dragging_slider = false;
        }

        // Ease the knob toward the selected minute. A quarter of the remaining
        // distance per frame is a critically-damped-looking approach that settles
        // in about six frames without ever overshooting.
        let target_q4 = y_from_minutes(self.minutes) << 4;
        if self.knob_q4 == 0 {
            self.knob_q4 = target_q4;
        } else {
            let delta = target_q4 - self.knob_q4;
            self.knob_q4 += if delta.abs() <= 16 { delta } else { delta / 4 };
        }

        // Edge-triggered; see `last_running`.
        let started = state.running && !self.last_running;
        let ended = !state.running && self.last_running;
        self.last_running = state.running;

        if self.wipe.is_none() {
            if started && self.screen != Screen::Running {
                self.start_wipe(Screen::Running, CX, CY, now_ms);
            }
            if ended && self.screen == Screen::Running {
                self.start_wipe(Screen::Home, CX, CY, now_ms);
            }
        }
    }

    /// True while the frame needs to keep being repainted. The countdown counts
    /// because its progress sweep is driven from fractional time, so it moves
    /// continuously rather than in one-second jumps.
    pub fn animating(&self) -> bool {
        self.wipe.is_some()
            || self.ripples.iter().any(|r| r.active)
            || self.screen == Screen::Running
            || self.dragging_slider
            || (self.screen == Screen::Force
                && self.knob_q4 != y_from_minutes(self.minutes) << 4)
    }

    /// Build the frame.
    ///
    /// Hit zones are registered on *every* frame for `interactive_screen()`,
    /// never only in particular animation phases. An earlier version registered
    /// them while drawing the fully-revealed screen only, so a wipe that
    /// finished between two repaints left the panel with no zones at all and
    /// nothing responded to touch until some later repaint happened along.
    pub fn build(&mut self, scene: &mut Scene, state: &State, now_ms: u32) {
        self.n_zones = 0;
        let base = self.screen;
        scene.clear(base.background());

        match self.wipe {
            None => self.draw_screen(scene, base, state, 255),
            Some(w) => {
                let age = now_ms.wrapping_sub(w.born_ms);
                if age < WIPE_COVER_MS {
                    // The outgoing screen stays visible under the growing disc,
                    // so the change reads as one surface covering another.
                    self.draw_screen(scene, base, state, 255);
                    let t = (age * 32_768 / WIPE_COVER_MS).min(32_768);
                    let eased = ease_out_q15(t);
                    let dx = w.x.max(W as i32 - w.x);
                    let dy = w.y.max(H as i32 - w.y);
                    let max_r = isqrt_i32(dx * dx + dy * dy) + 4;
                    let r = (max_r as u32 * eased / 32_768) as i32;
                    scene.disc(w.x, w.y, r, w.color, 255);
                } else {
                    scene.clear(w.color);
                    let reveal = (age - WIPE_COVER_MS).min(WIPE_REVEAL_MS);
                    let t = (reveal * 32_768 / WIPE_REVEAL_MS).min(32_768);
                    let alpha = (smoothstep_q15(t) * 255 / 32_768) as u8;
                    self.draw_screen(scene, w.to, state, alpha);
                }
            }
        }

        // One place registers zones, from the same constants the drawing uses.
        let interactive = self.interactive_screen();
        self.register(interactive, state);

        // Ripples ride above everything: they are feedback, not content.
        for i in 0..MAX_RIPPLES {
            let r = self.ripples[i];
            if !r.active {
                continue;
            }
            let age = now_ms.wrapping_sub(r.born_ms);
            let (radius, alpha) = if age < RIPPLE_GROW_MS {
                let t = age * 32_768 / RIPPLE_GROW_MS;
                ((r.max_r as u32 * ease_out_q15(t) / 32_768) as i32, 150u32)
            } else {
                let t = ((age - RIPPLE_GROW_MS) * 32_768 / RIPPLE_FADE_MS).min(32_768);
                (r.max_r, 150 * (32_768 - smoothstep_q15(t)) / 32_768)
            };
            // A soft expanding ring, so it reads as a ripple and never hides the
            // label underneath it.
            let thickness = (radius / 7).clamp(3, 16);
            scene.ring(r.x, r.y, radius, (radius - thickness).max(0), r.color, alpha as u8);
        }
    }

    /// Register touch targets for `screen`. Uses the same `l::` geometry the
    /// drawing does, so the two cannot disagree.
    fn register(&mut self, screen: Screen, state: &State) {
        let (bx, by, br) = l::BACK;
        match screen {
            Screen::Home => {
                let (x0, y0, x1, y1) = l::HOME_INSPECT;
                self.zone(Target::Inspect, Zone::Rect { x0, y0, x1, y1 });
                let (x0, y0, x1, y1) = l::HOME_FORCE;
                self.zone(Target::Force, Zone::Rect { x0, y0, x1, y1 });
            }
            Screen::Inspect => {
                self.zone(Target::Back, Zone::Disc { cx: bx, cy: by, r: br });
                for index in 0..state.n_starts.min(l::SCHED_MAX_ROWS) {
                    let cy = l::SCHED_FIRST_CY + index as i32 * l::SCHED_PITCH;
                    self.zone(
                        Target::Schedule(index),
                        Zone::Rect {
                            x0: l::SCHED_X0,
                            y0: cy - l::SCHED_HALF_H,
                            x1: l::SCHED_X1,
                            y1: cy + l::SCHED_HALF_H,
                        },
                    );
                }
            }
            Screen::Detail => {
                self.zone(Target::Back, Zone::Disc { cx: bx, cy: by, r: br });
            }
            Screen::Force => {
                self.zone(Target::Back, Zone::Disc { cx: bx, cy: by, r: br });
                self.zone(
                    Target::Slider,
                    Zone::Rect {
                        x0: l::SLIDER_X - l::SLIDER_HALF_W - 9,
                        y0: l::SLIDER_TOP - 20,
                        x1: l::SLIDER_X + l::SLIDER_HALF_W + 9,
                        y1: l::SLIDER_BOTTOM + 20,
                    },
                );
                let rows = state.n_usable().min(l::RELAY_MAX_ROWS);
                for index in 0..rows {
                    let cy = l::RELAY_FIRST_CY + index as i32 * l::RELAY_PITCH;
                    self.zone(
                        Target::Relay(index),
                        Zone::Rect {
                            x0: l::RELAY_X0,
                            y0: cy - l::RELAY_HALF_H,
                            x1: l::RELAY_X1,
                            y1: cy + l::RELAY_HALF_H,
                        },
                    );
                }
                let (gx, gy, gr) = l::GO;
                self.zone(Target::Go, Zone::Disc { cx: gx, cy: gy, r: gr });
            }
            Screen::Running => {
                self.zone(Target::Back, Zone::Disc { cx: bx, cy: by, r: br });
                let (x0, y0, x1, y1) = l::CANCEL;
                self.zone(Target::Cancel, Zone::Rect { x0, y0, x1, y1 });
            }
        }
    }

    fn draw_screen(&mut self, scene: &mut Scene, screen: Screen, state: &State, alpha: u8) {
        match screen {
            Screen::Home => self.draw_home(scene, state, alpha),
            Screen::Inspect => self.draw_inspect(scene, state, alpha),
            Screen::Force => self.draw_force(scene, state, alpha),
            Screen::Running => self.draw_running(scene, state, alpha),
            Screen::Detail => self.draw_detail(scene, state, alpha),
        }
    }

    fn draw_link(&self, scene: &mut Scene, state: &State, alpha: u8) {
        // One dot: green online, amber connecting, red offline. Small on
        // purpose - it matters only when it is wrong.
        let color = match state.link {
            Link::Online => C_RUN,
            Link::Connecting => C_FORCE,
            Link::Offline => C_CANCEL,
        };
        scene.disc(CX, 42, 7, color, alpha);
    }

    fn draw_back(&self, scene: &mut Scene, alpha: u8) {
        let (bx, by, br) = l::BACK;
        scene.ring(bx, by, br, br - 4, MUTED, alpha);
        // Chevron as two pills. The renderer cannot rotate a primitive, so the
        // arrow is a stem plus a shorter upright that reads as one mark at this
        // size.
        scene.pill(bx - 11, by - 3, bx + 10, by + 3, 3, INK, alpha);
        scene.pill(bx - 11, by - 12, bx - 5, by + 12, 3, INK, alpha);
    }

    fn draw_home(&mut self, scene: &mut Scene, state: &State, alpha: u8) {
        self.draw_link(scene, state, alpha);

        let mut clock = Buf::<8>::new();
        if state.clock_valid {
            let _ = write!(clock, "{:02}:{:02}", state.hh, state.mm);
        } else {
            let _ = write!(clock, "--:--");
        }
        scene.label(CX, 152, FontId::Display, INK, alpha, Align::Center, clock.as_str());

        let mut next = Buf::<40>::new();
        match state.next_start() {
            Some((s, minutes)) => {
                if minutes < 60 {
                    let _ = write!(next, "NEXT {:02}:{:02} IN {} MIN", s.hh, s.mm, minutes);
                } else {
                    let name = state
                        .relay_by_id(s.entries[0].relay as i32)
                        .map(|r| r.name)
                        .unwrap_or(Text::EMPTY);
                    let _ =
                        write!(next, "NEXT {:02}:{:02} \u{b7} {}", s.hh, s.mm, name.as_str());
                }
            }
            None => {
                let _ = write!(next, "NO SCHEDULE ARMED");
            }
        }
        scene.label(CX, 200, FontId::Caption, MUTED, alpha, Align::Center, next.as_str());

        // Two peers, same shape and size; colour and order carry the hierarchy.
        let (x0, y0, x1, y1) = l::HOME_INSPECT;
        scene.pill(x0, y0, x1, y1, l::HOME_PILL_R, C_INSPECT, alpha);
        scene.label(
            CX,
            (y0 + y1) / 2 + 14,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            "INSPECT",
        );

        let (x0, y0, x1, y1) = l::HOME_FORCE;
        scene.pill(x0, y0, x1, y1, l::HOME_PILL_R, C_FORCE, alpha);
        scene.label(
            CX,
            (y0 + y1) / 2 + 14,
            FontId::Body,
            rgb(26, 15, 2),
            alpha,
            Align::Center,
            "FORCE",
        );
    }

    fn draw_inspect(&mut self, scene: &mut Scene, state: &State, alpha: u8) {
        self.draw_back(scene, alpha);
        // Plural: this page lists every schedule, it is not one schedule's page.
        scene.label(CX, 116, FontId::Body, INK, alpha, Align::Center, "SCHEDULES");

        for index in 0..state.n_starts.min(l::SCHED_MAX_ROWS) {
            let s = state.starts[index];
            let on = s.enabled && s.n_entries > 0;
            let cy = l::SCHED_FIRST_CY + index as i32 * l::SCHED_PITCH;
            let (x0, x1) = (l::SCHED_X0, l::SCHED_X1);
            scene.pill(
                x0,
                cy - l::SCHED_HALF_H,
                x1,
                cy + l::SCHED_HALF_H,
                18,
                if on { rgb(18, 44, 92) } else { rgb(13, 22, 38) },
                alpha,
            );
            // Armed indicator.
            scene.disc(x0 + 28, cy, 9, if on { C_RUN } else { DIM }, alpha);

            let mut time = Buf::<8>::new();
            let _ = write!(time, "{:02}:{:02}", s.hh, s.mm);
            scene.label(
                x0 + 52,
                cy + 13,
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
            // Right-aligned clear of the chevron. The row is now the full width of
            // the panel, so this no longer has to fight the clock for space.
            scene.label(
                x1 - 44,
                cy + 10,
                FontId::Caption,
                if on { MUTED } else { DIM },
                alpha,
                Align::Right,
                summary.as_str(),
            );
            // Chevron, marking the row as something you can open.
            scene.pill(x1 - 26, cy - 8, x1 - 20, cy + 1, 3, MUTED, alpha);
            scene.pill(x1 - 26, cy - 1, x1 - 20, cy + 8, 3, MUTED, alpha);
        }

        // Sensor line. The controller reports several analog inputs; the first
        // record is I1, which is the one wired for this installation.
        if state.n_analogs > 0 {
            let a = state.analogs[0];
            let mut line = Buf::<28>::new();
            let _ = write!(line, "{} {} / 4095", a.name.as_str(), a.level);
            scene.label(CX, 452, FontId::Caption, DIM, alpha, Align::Center, line.as_str());
        }
    }

    /// One schedule's running order: which relay, for how long, in sequence.
    fn draw_detail(&mut self, scene: &mut Scene, state: &State, alpha: u8) {
        self.draw_back(scene, alpha);
        let index = self.detail.min(state.n_starts.saturating_sub(1));
        let s = state.starts[index];

        let mut title = Buf::<16>::new();
        let _ = write!(title, "{:02}:{:02}", s.hh, s.mm);
        scene.label(CX, 116, FontId::Display, INK, alpha, Align::Center, title.as_str());

        let mut sub = Buf::<28>::new();
        if s.enabled {
            let _ = write!(sub, "ARMED \u{b7} {} IN ORDER", s.n_entries);
        } else {
            let _ = write!(sub, "DISARMED");
        }
        scene.label(CX, 152, FontId::Caption, MUTED, alpha, Align::Center, sub.as_str());

        // Entries run top to bottom in the order the controller will drive them.
        let rows = s.n_entries.min(l::DETAIL_MAX_ROWS);
        for i in 0..rows {
            let e = s.entries[i];
            let cy = l::DETAIL_FIRST_CY + i as i32 * l::DETAIL_PITCH;
            scene.pill(
                l::SCHED_X0,
                cy - 21,
                l::SCHED_X1,
                cy + 21,
                18,
                rgb(15, 34, 72),
                alpha,
            );
            // Position in the running order.
            scene.disc(l::SCHED_X0 + 26, cy, 14, rgb(30, 70, 148), alpha);
            let mut n = Buf::<4>::new();
            let _ = write!(n, "{}", i + 1);
            scene.label(
                l::SCHED_X0 + 26,
                cy + 9,
                FontId::Caption,
                INK,
                alpha,
                Align::Center,
                n.as_str(),
            );

            let name = state.relay_by_id(e.relay as i32).map(|r| r.name).unwrap_or(Text::EMPTY);
            scene.label(
                l::SCHED_X0 + 52,
                cy + 9,
                FontId::Caption,
                INK,
                alpha,
                Align::Left,
                name.as_str(),
            );

            let mut dur = Buf::<12>::new();
            if e.seconds >= 60 {
                let _ = write!(dur, "{}:{:02}", e.seconds / 60, e.seconds % 60);
            } else {
                let _ = write!(dur, "{} S", e.seconds);
            }
            scene.label(
                l::SCHED_X1 - 22,
                cy + 9,
                FontId::Caption,
                MUTED,
                alpha,
                Align::Right,
                dur.as_str(),
            );
        }

        if s.n_entries > rows {
            let mut more = Buf::<20>::new();
            let _ = write!(more, "+{} MORE", s.n_entries - rows);
            scene.label(CX, 456, FontId::Caption, DIM, alpha, Align::Center, more.as_str());
        }
    }

    fn draw_force(&mut self, scene: &mut Scene, state: &State, alpha: u8) {
        self.draw_back(scene, alpha);

        let mut mins = Buf::<4>::new();
        let _ = write!(mins, "{}", self.minutes);
        scene.label(246, 158, FontId::Display, INK, alpha, Align::Right, mins.as_str());
        scene.label(258, 158, FontId::Caption, MUTED, alpha, Align::Left, "MIN");

        // Slider: track, filled portion below the knob, knob.
        let sx = l::SLIDER_X;
        let hw = l::SLIDER_HALF_W;
        scene.pill(
            sx - hw,
            l::SLIDER_TOP,
            sx + hw,
            l::SLIDER_BOTTOM,
            hw,
            rgb(58, 36, 8),
            alpha,
        );
        // Eased position, not the raw target - see `knob_q4`.
        let knob_y = (self.knob_q4 >> 4).clamp(l::SLIDER_TOP, l::SLIDER_BOTTOM);
        scene.pill(sx - hw, knob_y, sx + hw, l::SLIDER_BOTTOM, hw, C_FORCE, alpha);
        scene.disc(sx, knob_y, 28, rgb(252, 216, 154), alpha);

        for (index, relay) in state.usable().enumerate().take(l::RELAY_MAX_ROWS) {
            let selected = index == self.selected;
            let cy = l::RELAY_FIRST_CY + index as i32 * l::RELAY_PITCH;
            scene.pill(
                l::RELAY_X0,
                cy - l::RELAY_HALF_H,
                l::RELAY_X1,
                cy + l::RELAY_HALF_H,
                l::RELAY_HALF_H,
                if selected { C_FORCE } else { rgb(52, 34, 10) },
                alpha,
            );
            scene.label(
                (l::RELAY_X0 + l::RELAY_X1) / 2,
                cy + 9,
                FontId::Caption,
                if selected { rgb(26, 14, 0) } else { MUTED },
                alpha,
                Align::Center,
                relay.name.as_str(),
            );
        }

        let (gx, gy, gr) = l::GO;
        scene.disc(gx, gy, gr, C_RUN, alpha);
        scene.label(gx, gy + 16, FontId::Body, rgb(2, 22, 12), alpha, Align::Center, "GO!");
    }

    fn draw_running(&mut self, scene: &mut Scene, state: &State, alpha: u8) {
        self.draw_back(scene, alpha);

        // Progress gauge. Track first, then the elapsed sweep over it.
        //
        // Progress is computed from fractional seconds, not whole ones: driving
        // it from `left_s` alone moved the arc in one-second steps, which is what
        // made it look stuttery. With the sub-second remainder folded in, and the
        // countdown screen repainting continuously, the sweep is smooth.
        // Closed track, drawn as a ring rather than a full-turn arc: a ring needs
        // no angular test at all, so the cheap primitive does the cheap job.
        scene.ring(CX, CY, l::RING_OUTER, l::RING_INNER, rgb(10, 52, 36), alpha);

        let total_ms = (self.minutes * 60 * 1000).max(1);
        let left_ms = (state.left_s * 1000).saturating_sub(state.clock_frac_ms);
        let done_ms = total_ms.saturating_sub(left_ms.min(total_ms));
        if done_ms > 0 {
            // Scaled down before multiplying so the product stays inside u32.
            let denom = (total_ms / 8).max(1);
            let span = ((done_ms / 8).min(denom) * 4096 / denom) as i32;
            let span = span.clamp(0, 4095);
            if span > 0 {
                scene.arc(CX, CY, l::RING_OUTER, l::RING_INNER, 0, span, C_RUN, alpha);
            }
        }

        let name = state.relay_by_id(state.active).map(|r| r.name).unwrap_or(Text::EMPTY);
        scene.label(
            CX,
            168,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            if name.len > 0 { name.as_str() } else { "WATERING" },
        );

        // The countdown - the reason this screen exists, so it gets the largest
        // type on the device.
        let mut big = Buf::<10>::new();
        let _ = write!(big, "{}:{:02}", state.left_s / 60, state.left_s % 60);
        scene.label(CX, 320, FontId::Countdown, INK, alpha, Align::Center, big.as_str());

        let (x0, y0, x1, y1) = l::CANCEL;
        scene.pill(x0, y0, x1, y1, (y1 - y0) / 2, C_CANCEL, alpha);
        scene.label(CX, (y0 + y1) / 2 + 14, FontId::Body, INK, alpha, Align::Center, "CANCEL");

        if state.queued > 0 {
            let mut q = Buf::<24>::new();
            let _ = write!(q, "{} MORE QUEUED", state.queued);
            scene.label(CX, 446, FontId::Caption, MUTED, alpha, Align::Center, q.as_str());
        }
    }
}

/// Slider maps top = 10 minutes, bottom = 1 minute.
fn minutes_from_y(y: i32) -> u32 {
    let span = l::SLIDER_BOTTOM - l::SLIDER_TOP;
    let clamped = y.clamp(l::SLIDER_TOP, l::SLIDER_BOTTOM);
    let from_bottom = l::SLIDER_BOTTOM - clamped;
    (1 + (from_bottom * 9 + span / 2) / span).clamp(1, 10) as u32
}

fn y_from_minutes(minutes: u32) -> i32 {
    let span = l::SLIDER_BOTTOM - l::SLIDER_TOP;
    l::SLIDER_BOTTOM - ((minutes as i32 - 1) * span) / 9
}

#[inline]
fn smoothstep_q15(t: u32) -> u32 {
    let t = t.min(32_768);
    let squared = t * t >> 15;
    squared * (3 * 32_768 - 2 * t) >> 15
}

/// 1-(1-t)^3: fast off the mark, settles gently. Used for anything a finger just
/// launched, because it makes the response feel immediate.
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

// Keep the keep-out radius referenced so a future layout change trips the
// compiler rather than silently drifting outside the panel.
const _: () = {
    assert!(SAFE_R > 0);
};
