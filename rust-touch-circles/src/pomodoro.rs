//! A pomodoro timer you wind up.
//!
//! Two rings: drag round the outer one to set the work spell (5-60 minutes), the
//! inner one for the break (2-15). Tap the middle to start, again to pause. It
//! then alternates work and break by itself, with a long break - three times the
//! short one - after every fourth spell, which is the method as normally taught.
//!
//! Dragging round a ring needs the angle of a touch, and there is no atan2 here.
//! `turn_of` gets it from the octant plus a ratio inside that octant, which is
//! accurate to about a degree - far finer than a finger.

use crate::bubbles::Bubbles;
use crate::font::FontId;
use crate::gfx::{Align, H, Scene, TextBuf, W, muted, rgb};

const WORK_MIN: u32 = 5;
const WORK_MAX: u32 = 60;
const BREAK_MIN: u32 = 2;
const BREAK_MAX: u32 = 15;
/// Work spells before the long break.
const SET: u32 = 4;
const LONG_MULTIPLE: u32 = 3;

const R1_OUTER: i32 = 198;
const R1_INNER: i32 = 176;
const R2_OUTER: i32 = 168;
const R2_INNER: i32 = 150;

pub const CX: i32 = W as i32 / 2;
pub const CY: i32 = H as i32 / 2;
pub const RESET: (i32, i32, i32, i32) = (176, 366, 304, 416);

const C_WORK: u16 = rgb(255, 120, 90);
const C_BREAK: u16 = rgb(110, 220, 160);
const C_LONG: u16 = rgb(120, 190, 255);
const TRACK: u16 = rgb(28, 32, 38);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Work,
    Break,
    Long,
}

/// Which ring a touch landed on, if either.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Work,
    Break,
    Middle,
    Outside,
}

pub struct Pomodoro {
    work_min: u32,
    break_min: u32,
    phase: Phase,
    left_ms: u32,
    running: bool,
    done: u32,
    /// Which ring the current drag started on, so a wandering finger keeps
    /// adjusting the dial it began on.
    dragging: Option<Band>,
}

impl Pomodoro {
    pub const fn new() -> Self {
        Self {
            work_min: 25,
            break_min: 5,
            phase: Phase::Work,
            left_ms: 25 * 60 * 1000,
            running: false,
            done: 0,
            dragging: None,
        }
    }

    fn length_ms(&self, phase: Phase) -> u32 {
        match phase {
            Phase::Work => self.work_min * 60 * 1000,
            Phase::Break => self.break_min * 60 * 1000,
            Phase::Long => self.break_min * LONG_MULTIPLE * 60 * 1000,
        }
    }

    fn color(&self, phase: Phase) -> u16 {
        match phase {
            Phase::Work => C_WORK,
            Phase::Break => C_BREAK,
            Phase::Long => C_LONG,
        }
    }

    fn name(&self, phase: Phase) -> &'static str {
        match phase {
            Phase::Work => "FOCUS",
            Phase::Break => "BREAK",
            Phase::Long => "LONG BREAK",
        }
    }

    /// Back to the start of the current phase, keeping the dials and the tally.
    pub fn reset_phase(&mut self) {
        self.left_ms = self.length_ms(self.phase);
        self.running = false;
    }

    pub fn band_at(x: i32, y: i32) -> Band {
        let (dx, dy) = (x - CX, y - CY);
        let distance = ((dx * dx + dy * dy) as u32).isqrt() as i32;
        if distance > R1_OUTER + 8 {
            Band::Outside
        } else if distance >= R1_INNER - 4 {
            Band::Work
        } else if distance >= R2_INNER - 4 {
            Band::Break
        } else {
            Band::Middle
        }
    }

    pub fn press(&mut self, x: i32, y: i32) {
        let band = Self::band_at(x, y);
        match band {
            // The dials only move while stopped: winding one mid-spell would be a
            // way to lose track of the time you had already put in.
            Band::Work | Band::Break if !self.running => {
                self.dragging = Some(band);
                self.wind(band, x, y);
            }
            Band::Middle => self.running = !self.running,
            _ => {}
        }
    }

    pub fn drag(&mut self, x: i32, y: i32) {
        if let Some(band) = self.dragging {
            self.wind(band, x, y);
        }
    }

    pub fn release(&mut self) {
        self.dragging = None;
    }

    fn wind(&mut self, band: Band, x: i32, y: i32) {
        let turn = turn_of(x - CX, y - CY);
        // A whole turn spans the dial's range, so the wind-up reads as a clock.
        match band {
            Band::Work => {
                let span = WORK_MAX - WORK_MIN;
                self.work_min = WORK_MIN + (turn as u32 * span + 2048) / 4096;
                self.work_min = self.work_min.clamp(WORK_MIN, WORK_MAX);
            }
            Band::Break => {
                let span = BREAK_MAX - BREAK_MIN;
                self.break_min = BREAK_MIN + (turn as u32 * span + 2048) / 4096;
                self.break_min = self.break_min.clamp(BREAK_MIN, BREAK_MAX);
            }
            _ => return,
        }
        // Winding while stopped also sets the clock, so what you see is what will
        // run.
        if !self.running {
            self.left_ms = self.length_ms(self.phase);
        }
    }

    pub fn update(&mut self, dt_ms: u32, now_ms: u32, bubbles: &mut Bubbles) {
        if !self.running {
            return;
        }
        self.left_ms = self.left_ms.saturating_sub(dt_ms.min(200));
        if self.left_ms > 0 {
            return;
        }

        bubbles.spawn(
            CX,
            CY,
            now_ms,
            Some(muted(self.color(self.phase))),
            None,
            true,
        );
        self.phase = match self.phase {
            Phase::Work => {
                self.done += 1;
                if self.done % SET == 0 {
                    Phase::Long
                } else {
                    Phase::Break
                }
            }
            _ => Phase::Work,
        };
        self.left_ms = self.length_ms(self.phase);
        // Paused between phases: a break should not start eating itself while the
        // kettle is on.
        self.running = false;
    }

    pub fn draw(&self, scene: &mut Scene, alpha: u8) {
        let live = self.color(self.phase);

        // Each ring shows its own setting, and the one that is running shows how
        // much of it is left instead.
        for (outer, inner, phase, span_of) in [
            (
                R1_OUTER,
                R1_INNER,
                Phase::Work,
                fraction(self.work_min - WORK_MIN, WORK_MAX - WORK_MIN),
            ),
            (
                R2_OUTER,
                R2_INNER,
                Phase::Break,
                fraction(self.break_min - BREAK_MIN, BREAK_MAX - BREAK_MIN),
            ),
        ] {
            scene.ring(CX, CY, outer, inner, TRACK, alpha);
            let running_here = self.running
                && (phase == self.phase
                    || (phase == Phase::Break && self.phase == Phase::Long));
            let (span, color) = if running_here {
                let total = self.length_ms(self.phase).max(1);
                let done = total - self.left_ms.min(total);
                ((done as u64 * 4096 / total as u64) as i32, live)
            } else if self.running {
                (span_of, muted(self.color(phase)))
            } else {
                (span_of, self.color(phase))
            };
            if span > 0 {
                scene.arc(CX, CY, outer, inner, 0, span, color, alpha);
            }
        }

        scene.label(
            CX,
            CY - 92,
            FontId::Caption,
            live,
            alpha,
            Align::Center,
            self.name(self.phase),
        );

        let seconds = (self.left_ms + 999) / 1000;
        let mut clock = TextBuf::new();
        use core::fmt::Write as _;
        let _ = write!(clock, "{}:{:02}", seconds / 60, seconds % 60);
        scene.label(
            CX,
            CY + 46,
            FontId::Countdown,
            rgb(238, 245, 250),
            alpha,
            Align::Center,
            clock.as_str(),
        );

        // The two settings, so the dials can be read as numbers as well.
        let mut dials = TextBuf::new();
        let _ = write!(dials, "{} MIN \u{b7} {} BREAK", self.work_min, self.break_min);
        scene.label(
            CX,
            CY + 86,
            FontId::Micro,
            rgb(150, 160, 172),
            alpha,
            Align::Center,
            dials.as_str(),
        );

        let filled = self.done % SET;
        for index in 0..SET {
            let x = CX - 30 + index as i32 * 20;
            let lit = index < filled;
            scene.disc(
                x,
                CY + 118,
                if lit { 6 } else { 4 },
                if lit { live } else { TRACK },
                alpha,
            );
        }

        let (x0, y0, x1, y1) = RESET;
        scene.pill(x0, y0, x1, y1, (y1 - y0) / 2, rgb(26, 30, 36), alpha);
        scene.label(
            (x0 + x1) / 2,
            (y0 + y1) / 2 + 9,
            FontId::Caption,
            rgb(150, 160, 172),
            alpha,
            Align::Center,
            "RESET",
        );
    }
}

fn fraction(value: u32, span: u32) -> i32 {
    (value * 4096 / span.max(1)) as i32
}

/// Turn of a vector, Q12 (4096 to the circle), clockwise from twelve o'clock.
///
/// Octant plus a linear step inside it: eight cases, no trigonometry, and within
/// about a degree of the truth - which is finer than a fingertip on a 200 px ring.
fn turn_of(dx: i32, dy: i32) -> i32 {
    let (u, v) = (dx, -dy);
    let (au, av) = (u.abs(), v.abs());
    if au == 0 && av == 0 {
        return 0;
    }
    // Within one octant, the ratio of the shorter leg to the longer is close
    // enough to linear in the angle for a control like this.
    let step = if au <= av {
        au * 512 / av.max(1)
    } else {
        1024 - av * 512 / au.max(1)
    };
    match (u >= 0, v >= 0) {
        (true, true) => step,
        (true, false) => 2048 - step,
        (false, false) => 2048 + step,
        (false, true) => 4096 - step,
    }
}
