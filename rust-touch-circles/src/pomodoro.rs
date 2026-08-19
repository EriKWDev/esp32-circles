//! A pomodoro timer, wearing the relay countdown's clothes: one big ring, one big
//! number. Tap the middle to start or pause; the button below resets the phase.
//!
//! Phases advance by themselves - four spells of work, each followed by a short
//! break, then a long one - so the only decision left is when to begin.

use crate::bubbles::Bubbles;
use crate::font::FontId;
use crate::gfx::{Align, H, Scene, TextBuf, W, muted, rgb};

const WORK_MS: u32 = 25 * 60 * 1000;
const SHORT_MS: u32 = 5 * 60 * 1000;
const LONG_MS: u32 = 15 * 60 * 1000;
/// Work spells before the long break.
const SET: u32 = 4;

const RING_OUTER: i32 = 196;
const RING_INNER: i32 = 178;
/// (cx, cy, r) for the tap-to-start target, which is the whole dial.
pub const DIAL: (i32, i32, i32) = (W as i32 / 2, H as i32 / 2, RING_INNER);
pub const RESET: (i32, i32, i32, i32) = (170, 372, 310, 424);

const C_WORK: u16 = rgb(255, 120, 90);
const C_SHORT: u16 = rgb(110, 220, 160);
const C_LONG: u16 = rgb(120, 190, 255);
const TRACK: u16 = rgb(30, 34, 40);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Work,
    Short,
    Long,
}

impl Phase {
    fn length(self) -> u32 {
        match self {
            Phase::Work => WORK_MS,
            Phase::Short => SHORT_MS,
            Phase::Long => LONG_MS,
        }
    }

    fn color(self) -> u16 {
        match self {
            Phase::Work => C_WORK,
            Phase::Short => C_SHORT,
            Phase::Long => C_LONG,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Phase::Work => "FOCUS",
            Phase::Short => "BREAK",
            Phase::Long => "LONG BREAK",
        }
    }
}

pub struct Pomodoro {
    phase: Phase,
    left_ms: u32,
    running: bool,
    /// Completed work spells, which is what decides when the long break is due.
    done: u32,
}

impl Pomodoro {
    pub const fn new() -> Self {
        Self {
            phase: Phase::Work,
            left_ms: WORK_MS,
            running: false,
            done: 0,
        }
    }

    /// Only resets the clock, not the tally: leaving the page and coming back
    /// should not lose count of the afternoon.
    pub fn reset_phase(&mut self) {
        self.left_ms = self.phase.length();
        self.running = false;
    }

    pub fn toggle(&mut self) {
        self.running = !self.running;
    }

    pub fn update(&mut self, dt_ms: u32, now_ms: u32, bubbles: &mut Bubbles) {
        if !self.running {
            return;
        }
        self.left_ms = self.left_ms.saturating_sub(dt_ms.min(200));
        if self.left_ms > 0 {
            return;
        }

        // A ring in the finished phase's colour, then straight into the next one -
        // paused, so a break cannot start eating itself while you are away.
        bubbles.spawn(
            W as i32 / 2,
            H as i32 / 2,
            now_ms,
            Some(muted(self.phase.color())),
            None,
            true,
        );
        self.phase = match self.phase {
            Phase::Work => {
                self.done += 1;
                if self.done % SET == 0 {
                    Phase::Long
                } else {
                    Phase::Short
                }
            }
            _ => Phase::Work,
        };
        self.left_ms = self.phase.length();
        self.running = false;
    }

    pub fn draw(&self, scene: &mut Scene, alpha: u8) {
        let (cx, cy, _) = DIAL;
        let color = self.phase.color();
        scene.ring(cx, cy, RING_OUTER, RING_INNER, TRACK, alpha);

        let total = self.phase.length().max(1);
        let done = total - self.left_ms.min(total);
        let span = (done as u64 * 4096 / total as u64) as i32;
        if span > 0 {
            scene.arc(cx, cy, RING_OUTER, RING_INNER, 0, span, color, alpha);
        }

        scene.label(
            cx,
            cy - 96,
            FontId::Caption,
            color,
            alpha,
            Align::Center,
            self.phase.name(),
        );

        // Rounded up, so a running timer never shows 0:00 with time left.
        let seconds = (self.left_ms + 999) / 1000;
        let mut clock = TextBuf::new();
        use core::fmt::Write as _;
        let _ = write!(clock, "{}:{:02}", seconds / 60, seconds % 60);
        scene.label(
            cx,
            cy + 52,
            FontId::Countdown,
            rgb(238, 245, 250),
            alpha,
            Align::Center,
            clock.as_str(),
        );

        // Which spell of the set this is, as dots.
        let filled = self.done % SET;
        for index in 0..SET {
            let x = cx - 30 + index as i32 * 20;
            let lit = index < filled;
            scene.disc(
                x,
                cy + 96,
                if lit { 6 } else { 4 },
                if lit { color } else { TRACK },
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

        scene.label(
            cx,
            H as i32 - 26,
            FontId::Micro,
            rgb(90, 100, 112),
            alpha,
            Align::Center,
            if self.running { "TAP TO PAUSE" } else { "TAP TO START" },
        );
    }
}
