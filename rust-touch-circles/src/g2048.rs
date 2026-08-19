//! 2048. Swipe to slide.
//!
//! A move is resolved immediately into the next board, but each tile remembers
//! where it came from and is drawn along that path for SLIDE_MS - so the
//! animation is a presentation of a decision already made, and input can never
//! land on a half-moved grid. Merges pop: the tile overshoots its size briefly,
//! which is what makes a merge feel like an event.

use crate::bubbles::Bubbles;
use crate::font::FontId;
use crate::gfx::{Align, Scene, TextBuf, W, muted, rgb};

const N: usize = 4;
const MARGIN: i32 = 22;
const BOARD_TOP: i32 = 104;
const CELL: i32 = (W as i32 - 2 * MARGIN) / N as i32;
const PAD: i32 = 5;

const SLIDE_MS: u32 = 130;
const POP_MS: u32 = 160;

const BOARD_BG: u16 = rgb(26, 24, 22);
const CELL_BG: u16 = rgb(38, 35, 32);

/// Warm and increasingly loud with the exponent, which is what makes progress
/// legible at a glance.
fn tile_color(value: u32) -> (u16, u16) {
    match value {
        2 => (rgb(238, 228, 218), rgb(60, 56, 50)),
        4 => (rgb(237, 224, 200), rgb(60, 56, 50)),
        8 => (rgb(242, 177, 121), rgb(255, 250, 245)),
        16 => (rgb(245, 149, 99), rgb(255, 250, 245)),
        32 => (rgb(246, 124, 95), rgb(255, 250, 245)),
        64 => (rgb(246, 94, 59), rgb(255, 250, 245)),
        128 => (rgb(237, 207, 114), rgb(255, 252, 240)),
        256 => (rgb(237, 204, 97), rgb(255, 252, 240)),
        512 => (rgb(237, 200, 80), rgb(255, 252, 240)),
        1024 => (rgb(120, 210, 190), rgb(255, 255, 255)),
        _ => (rgb(90, 190, 220), rgb(255, 255, 255)),
    }
}

#[derive(Clone, Copy)]
struct Moving {
    value: u32,
    from: (i32, i32),
    to: (usize, usize),
    merged: bool,
}

const NO_MOVE: Moving = Moving {
    value: 0,
    from: (0, 0),
    to: (0, 0),
    merged: false,
};

pub struct G2048 {
    grid: [[u32; N]; N],
    /// The tiles of the move being presented, and when it started.
    moves: [Moving; N * N],
    n_moves: usize,
    anim_start_ms: u32,
    score: u32,
    best: u32,
    /// A tile appears only once its slide has finished, so it does not seem to
    /// have been there all along.
    pending_spawn: Option<(usize, usize, u32)>,
}

impl G2048 {
    pub const fn new() -> Self {
        Self {
            grid: [[0; N]; N],
            moves: [NO_MOVE; N * N],
            n_moves: 0,
            anim_start_ms: 0,
            score: 0,
            best: 0,
            pending_spawn: None,
        }
    }

    pub fn restart(&mut self, now_ms: u32) {
        let best = self.best.max(self.score);
        *self = Self::new();
        self.best = best;
        self.place(now_ms);
        self.place(now_ms + 7);
    }

    /// A 2 (or occasionally a 4) in a free cell, chosen off the clock. Returns
    /// what it chose without applying it, so a move can decide now and reveal
    /// later.
    fn choose_free(&self, seed: u32) -> Option<(usize, usize, u32)> {
        let free = self.grid.iter().flatten().filter(|v| **v == 0).count();
        if free == 0 {
            return None;
        }
        let pick = (seed as usize / 3) % free;
        let mut seen = 0;
        for r in 0..N {
            for c in 0..N {
                if self.grid[r][c] != 0 {
                    continue;
                }
                if seen == pick {
                    return Some((r, c, if seed % 10 == 0 { 4 } else { 2 }));
                }
                seen += 1;
            }
        }
        None
    }

    fn place(&mut self, seed: u32) {
        if let Some((r, c, value)) = self.choose_free(seed) {
            self.grid[r][c] = value;
        }
    }

    /// True while a slide is still being drawn; input is ignored until it ends.
    fn animating(&self, now_ms: u32) -> bool {
        self.n_moves > 0 && now_ms.wrapping_sub(self.anim_start_ms) < SLIDE_MS
    }

    pub fn update(&mut self, now_ms: u32, bubbles: &mut Bubbles) {
        if self.n_moves > 0 && !self.animating(now_ms) {
            // The slide is over: reveal the new tile and put the merges on screen
            // as rings, which is the only celebration this board needs.
            for index in 0..self.n_moves {
                let m = self.moves[index];
                if !m.merged {
                    continue;
                }
                let (x, y) = cell_centre(m.to.0, m.to.1);
                bubbles.spawn(
                    x,
                    y,
                    now_ms,
                    Some(muted(tile_color(m.value).0)),
                    Some(CELL * 3 / 2),
                    true,
                );
            }
            if let Some((r, c, value)) = self.pending_spawn.take() {
                self.grid[r][c] = value;
            }
            self.n_moves = 0;
        }
    }

    /// `dx`/`dy` are the swipe: exactly one is non-zero.
    pub fn swipe(&mut self, dx: i32, dy: i32, now_ms: u32) {
        if self.animating(now_ms) {
            return;
        }
        let before = self.grid;
        self.n_moves = 0;

        // One implementation, four directions: walk each line in the direction of
        // travel and compact it. `line` maps a position along a line to a cell, so
        // the merge logic never has to know which way it is facing.
        let horizontal = dx != 0;
        let forward = dx > 0 || dy > 0;
        for major in 0..N {
            let mut slot: i32 = if forward { N as i32 - 1 } else { 0 };
            let step: i32 = if forward { -1 } else { 1 };
            let mut last_merge_slot: i32 = -100;
            let order: [usize; N] = if forward { [3, 2, 1, 0] } else { [0, 1, 2, 3] };
            for &along in &order {
                let (r, c) = if horizontal { (major, along) } else { (along, major) };
                let value = before[r][c];
                if value == 0 {
                    continue;
                }
                let (pr, pc) = if horizontal {
                    (major, slot as usize)
                } else {
                    (slot as usize, major)
                };
                // Merge into the tile just placed when it matches and has not
                // already absorbed something this move.
                let prev_slot = slot - step;
                let (qr, qc) = if horizontal {
                    (major, prev_slot.clamp(0, N as i32 - 1) as usize)
                } else {
                    (prev_slot.clamp(0, N as i32 - 1) as usize, major)
                };
                if prev_slot >= 0
                    && prev_slot < N as i32
                    && self.grid[qr][qc] == value
                    && prev_slot != last_merge_slot
                {
                    self.grid[qr][qc] = value * 2;
                    self.score += value * 2;
                    last_merge_slot = prev_slot;
                    self.record(value * 2, (r, c), (qr, qc), true);
                    continue;
                }
                self.grid[pr][pc] = value;
                self.record(value, (r, c), (pr, pc), false);
                slot += step;
            }
        }

        // Clear whatever the compaction left behind, then decide if anything moved.
        for r in 0..N {
            for c in 0..N {
                if !self
                    .moves
                    .iter()
                    .take(self.n_moves)
                    .any(|m| m.to == (r, c))
                {
                    self.grid[r][c] = 0;
                }
            }
        }
        if self.grid == before {
            self.n_moves = 0;
            return;
        }

        self.anim_start_ms = now_ms;
        self.best = self.best.max(self.score);
        // Chosen now, revealed once the slide has landed.
        self.pending_spawn = self.choose_free(now_ms);
    }

    fn record(&mut self, value: u32, from: (usize, usize), to: (usize, usize), merged: bool) {
        if self.n_moves >= self.moves.len() {
            return;
        }
        let (fx, fy) = cell_centre(from.0, from.1);
        self.moves[self.n_moves] = Moving {
            value,
            from: (fx, fy),
            to,
            merged,
        };
        self.n_moves += 1;
    }

    pub fn draw(&self, scene: &mut Scene, now_ms: u32, alpha: u8) {
        scene.pill(
            MARGIN - 6,
            BOARD_TOP - 6,
            W as i32 - MARGIN + 6,
            BOARD_TOP + N as i32 * CELL + 6,
            12,
            BOARD_BG,
            alpha,
        );
        for r in 0..N {
            for c in 0..N {
                let (x, y) = cell_centre(r, c);
                scene.pill(
                    x - CELL / 2 + PAD,
                    y - CELL / 2 + PAD,
                    x + CELL / 2 - PAD,
                    y + CELL / 2 - PAD,
                    8,
                    CELL_BG,
                    alpha,
                );
            }
        }

        let sliding = self.animating(now_ms);
        let t = if sliding {
            (now_ms.wrapping_sub(self.anim_start_ms) * 256 / SLIDE_MS).min(256) as i32
        } else {
            256
        };

        if sliding {
            for m in self.moves.iter().take(self.n_moves) {
                let (tx, ty) = cell_centre(m.to.0, m.to.1);
                let x = m.from.0 + (tx - m.from.0) * t / 256;
                let y = m.from.1 + (ty - m.from.1) * t / 256;
                self.tile(scene, x, y, CELL / 2 - PAD, m.value, alpha);
            }
            return;
        }

        for r in 0..N {
            for c in 0..N {
                let value = self.grid[r][c];
                if value == 0 {
                    continue;
                }
                let (x, y) = cell_centre(r, c);
                // A merged tile overshoots for a moment after landing.
                let age = now_ms.wrapping_sub(self.anim_start_ms + SLIDE_MS);
                let popped = self
                    .moves
                    .iter()
                    .take(self.n_moves)
                    .any(|m| m.merged && m.to == (r, c));
                let mut half = CELL / 2 - PAD;
                if popped && age < POP_MS {
                    let phase = (age * 256 / POP_MS) as i32;
                    // Out and back: +8% at the middle of the pop.
                    let bump = if phase < 128 { phase } else { 256 - phase };
                    half += half * bump / 1600;
                }
                self.tile(scene, x, y, half, value, alpha);
            }
        }
    }

    fn tile(&self, scene: &mut Scene, x: i32, y: i32, half: i32, value: u32, alpha: u8) {
        let (plate, ink) = tile_color(value);
        scene.pill(x - half, y - half, x + half, y + half, 8, plate, alpha);
        let mut text = TextBuf::new();
        use core::fmt::Write as _;
        let _ = write!(text, "{value}");
        // Four-digit numbers need the smaller face to fit the cell.
        let font = if value >= 1000 {
            FontId::Caption
        } else {
            FontId::Body
        };
        let f = font.get();
        scene.label(
            x,
            y + f.ascent / 2 - f.ascent / 8,
            font,
            ink,
            alpha,
            Align::Center,
            text.as_str(),
        );
    }

    pub fn draw_header(&self, scene: &mut Scene, alpha: u8) {
        let mut line = TextBuf::new();
        use core::fmt::Write as _;
        let _ = write!(line, "{} \u{b7} BEST {}", self.score, self.best);
        scene.label(
            W as i32 / 2,
            76,
            FontId::Caption,
            rgb(226, 220, 210),
            alpha,
            Align::Center,
            line.as_str(),
        );
    }
}

fn cell_centre(row: usize, col: usize) -> (i32, i32) {
    (
        MARGIN + col as i32 * CELL + CELL / 2,
        BOARD_TOP + row as i32 * CELL + CELL / 2,
    )
}
