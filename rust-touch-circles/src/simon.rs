//! Simon says. Watch the pattern, then repeat it.
//!
//! Each pad owns a note from the pentatonic set in `audio`, so any pattern sounds
//! like a tune rather than a series of beeps. A round adds one step and replays
//! the whole sequence; one wrong tap ends it.
//!
//! The notes are sounded by the caller, not from here, because playing one blocks
//! (see `audio`). This owns the pattern and the state machine and says which pad
//! is due; the UI makes the noise.

use crate::font::FontId;
use crate::gfx::{Align, Scene, TextBuf, W, muted, rgb};

pub const MAX_STEPS: usize = 32;
/// Quiet between the notes of a pattern. Without it the sequence runs together
/// and cannot be memorised, which is the entire game.
const GAP_MS: u32 = 230;
/// Pause before a pattern begins, so the start is never a surprise.
const LEAD_MS: u32 = 650;
/// How long every pad stays lit after a round is completed.
const FLASH_MS: u32 = 700;
pub const COLORS: [u16; 4] = [
    rgb(255, 96, 110),
    rgb(120, 210, 255),
    rgb(255, 205, 90),
    rgb(140, 230, 160),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Ready,
    Showing { at: usize },
    Input { at: usize },
    /// Round complete: all four lit while the win tune plays.
    Won,
    Lost,
}

pub enum Answer {
    Nothing,
    Pad(usize),
    Round,
    Lost,
}

pub struct Simon {
    steps: [u8; MAX_STEPS],
    len: usize,
    pub phase: Phase,
    lit: Option<(usize, u32)>,
    /// Earliest the next thing may happen, which is what paces the whole game.
    next_at_ms: u32,
    rng: u32,
    pub best: usize,
}

impl Simon {
    pub const fn new() -> Self {
        Self {
            steps: [0; MAX_STEPS],
            len: 0,
            phase: Phase::Ready,
            lit: None,
            next_at_ms: 0,
            rng: 0x1234_5678,
            best: 0,
        }
    }

    pub fn restart(&mut self, now_ms: u32) {
        let best = self.best;
        *self = Self::new();
        self.best = best;
        self.rng = now_ms | 1;
        self.extend();
        self.phase = Phase::Showing { at: 0 };
        self.next_at_ms = now_ms + LEAD_MS;
    }

    fn next_rand(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    fn extend(&mut self) {
        if self.len < MAX_STEPS {
            self.steps[self.len] = (self.next_rand() % 4) as u8;
            self.len += 1;
        }
    }

    /// The pad to light and sound now, while the pattern plays.
    pub fn pending_show(&self, now_ms: u32) -> Option<usize> {
        if now_ms < self.next_at_ms {
            return None;
        }
        match self.phase {
            Phase::Showing { at } if at < self.len => Some(self.steps[at] as usize),
            _ => None,
        }
    }

    /// Called once that step has been sounded.
    pub fn advance_show(&mut self, now_ms: u32) {
        if let Phase::Showing { at } = self.phase {
            self.lit = Some((self.steps[at] as usize, now_ms + 120));
            self.next_at_ms = now_ms + GAP_MS;
            self.phase = if at + 1 < self.len {
                Phase::Showing { at: at + 1 }
            } else {
                Phase::Input { at: 0 }
            };
        }
    }

    pub fn tap(&mut self, pad: usize, now_ms: u32) -> Answer {
        self.lit = Some((pad, now_ms + 140));
        match self.phase {
            // Nothing to do mid-celebration; `update` moves it along.
            Phase::Won => Answer::Nothing,
            Phase::Ready | Phase::Lost => {
                self.restart(now_ms);
                Answer::Nothing
            }
            Phase::Input { at } => {
                if self.steps[at] as usize != pad {
                    self.phase = Phase::Lost;
                    self.best = self.best.max(self.len.saturating_sub(1));
                    return Answer::Lost;
                }
                if at + 1 < self.len {
                    self.phase = Phase::Input { at: at + 1 };
                    Answer::Pad(pad)
                } else {
                    self.best = self.best.max(self.len);
                    self.phase = Phase::Won;
                    self.next_at_ms = now_ms + FLASH_MS;
                    Answer::Round
                }
            }
            Phase::Showing { .. } => Answer::Nothing,
        }
    }

    pub fn pad_at(x: i32, y: i32) -> Option<usize> {
        (0..4).find(|pad| {
            let (x0, y0, x1, y1) = Self::pad_rect(*pad);
            x >= x0 && x <= x1 && y >= y0 && y <= y1
        })
    }

    /// Sized by the room below the caption and centred - sizing from the width put
    /// the bottom row off the screen entirely.
    pub fn pad_rect(pad: usize) -> (i32, i32, i32, i32) {
        const GAP: i32 = 12;
        const TOP: i32 = 116;
        const SIZE: i32 = 168;
        let margin = (W as i32 - (2 * SIZE + GAP)) / 2;
        let col = (pad % 2) as i32;
        let row = (pad / 2) as i32;
        let x0 = margin + col * (SIZE + GAP);
        let y0 = TOP + row * (SIZE + GAP);
        (x0, y0, x0 + SIZE, y0 + SIZE)
    }

    pub fn update(&mut self, now_ms: u32) {
        if let Some((_, until)) = self.lit {
            if now_ms >= until {
                self.lit = None;
            }
        }
        // The celebration ends by itself, and only then is the pattern extended -
        // so the new step is not visible behind the flash.
        if self.phase == Phase::Won && now_ms >= self.next_at_ms {
            self.extend();
            self.phase = Phase::Showing { at: 0 };
            self.next_at_ms = now_ms + LEAD_MS;
        }
    }

    pub fn draw(&self, scene: &mut Scene, alpha: u8) {
        for pad in 0..4 {
            let (x0, y0, x1, y1) = Self::pad_rect(pad);
            // Every pad lights for the round-complete flash.
            let lit = self.phase == Phase::Won
                || self.lit.is_some_and(|(which, _)| which == pad);
            // Dim until lit, so the pattern is the only thing moving.
            let color = if lit {
                COLORS[pad]
            } else {
                muted(muted(COLORS[pad]))
            };
            scene.pill(x0, y0, x1, y1, 18, color, alpha);
        }

        let mut line = TextBuf::new();
        use core::fmt::Write as _;
        match self.phase {
            Phase::Ready => {
                let _ = write!(line, "TAP TO START");
            }
            Phase::Showing { .. } => {
                let _ = write!(line, "WATCH {}", self.len);
            }
            Phase::Input { at } => {
                let _ = write!(line, "{} OF {}", at + 1, self.len);
            }
            Phase::Won => {
                let _ = write!(line, "{} RIGHT", self.len);
            }
            Phase::Lost => {
                let _ = write!(line, "MISSED - BEST {}", self.best);
            }
        }
        scene.label(
            W as i32 / 2,
            88,
            FontId::Caption,
            rgb(228, 234, 240),
            alpha,
            Align::Center,
            line.as_str(),
        );
    }
}
