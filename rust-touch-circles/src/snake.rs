//! Masken: the Nokia worm. Eat, grow, and do not bite yourself.
//!
//! Touch where you want to go: the panel is split into four wedges around the
//! board's centre, so press the top half's middle to go up, the left to go left.
//! The whole screen steers and every target is a quarter of it - a swipe was the
//! first attempt and read as confusing, because a gesture gives no clue which way
//! it thinks you meant.
//!
//! The grid is coarse on purpose - a fingertip is about one cell wide, and the
//! original was coarser still.
//!
//! The body is drawn as merged runs rather than a disc per cell. That is not only
//! tidier - straight lengths become one rounded bar, which is how the phone drew
//! it - it is what keeps a long worm inside the scene's primitive budget: a
//! hundred cells is typically a dozen runs, where a hundred discs would overflow
//! it and render half a worm.

use crate::bubbles::Bubbles;
use crate::font::FontId;
use crate::gfx::{Align, H, Scene, TextBuf, W, muted, rgb};

const CELL: i32 = 28;
const COLS: i32 = 16;
const ROWS: i32 = 13;
/// Below the score line, and the whole board fits above the bottom edge - which
/// is worth stating because sizing a board from the screen and then pushing it
/// down under a header is how boards end up off-screen.
const BOARD_X: i32 = (W as i32 - COLS * CELL) / 2;
const BOARD_Y: i32 = 92;
const BOARD_W: i32 = COLS * CELL;
const BOARD_H: i32 = ROWS * CELL;

const MAX_LEN: usize = (COLS * ROWS) as usize;
const START_LEN: usize = 4;
/// Milliseconds per step at the start, and the floor it accelerates towards.
const STEP_MS: u32 = 190;
const STEP_MIN_MS: u32 = 85;
/// Taken off the step for each apple eaten.
const STEP_GAIN_MS: u32 = 4;

const C_BODY: u16 = rgb(120, 230, 140);
const C_HEAD: u16 = rgb(200, 255, 200);
const C_FOOD: u16 = rgb(255, 110, 90);
const C_BOARD: u16 = rgb(12, 18, 14);
const C_EDGE: u16 = rgb(30, 44, 34);
const INK: u16 = rgb(238, 245, 250);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for a tap, both at the start and after a death.
    Ready,
    Run,
    Over,
}

pub struct Snake {
    /// Ring buffer of cells, newest at `head`.
    body: [(i8, i8); MAX_LEN],
    head: usize,
    len: usize,
    dx: i8,
    dy: i8,
    /// Turn taken this step. The direction only changes once per step, so a fast
    /// double swipe cannot fold the worm back into itself between moves.
    pending: Option<(i8, i8)>,
    food: (i8, i8),
    phase: Phase,
    score: u32,
    pub best: u32,
    step_ms: u32,
    next_step_ms: u32,
    seed: u32,
}

impl Snake {
    pub const fn new() -> Self {
        Self {
            body: [(0, 0); MAX_LEN],
            head: 0,
            len: START_LEN,
            dx: 1,
            dy: 0,
            pending: None,
            food: (COLS as i8 - 4, ROWS as i8 / 2),
            phase: Phase::Ready,
            score: 0,
            best: 0,
            step_ms: STEP_MS,
            next_step_ms: 0,
            seed: 0x1234_5678,
        }
    }

    pub fn restart(&mut self, now_ms: u32) {
        let best = self.best;
        *self = Self::new();
        self.best = best;
        self.seed ^= now_ms | 1;
        // Laid out along the middle row, head to the right.
        for index in 0..START_LEN {
            self.body[index] = ((COLS / 2 - index as i32) as i8, (ROWS / 2) as i8);
        }
        self.head = 0;
        self.next_step_ms = now_ms + self.step_ms;
        self.place_food();
    }

    /// A tap starts a waiting game, or restarts a finished one. Anywhere on the
    /// screen: there is nothing else to hit.
    pub fn tap(&mut self, now_ms: u32) {
        match self.phase {
            Phase::Ready => {
                self.phase = Phase::Run;
                self.next_step_ms = now_ms + self.step_ms;
            }
            Phase::Over => self.restart(now_ms),
            Phase::Run => {}
        }
    }

    /// Steer towards the touch. The wedge is decided by which offset from the
    /// board's centre is the larger, so the four regions are triangles meeting at
    /// the middle and between them they cover the panel.
    pub fn steer(&mut self, x: i32, y: i32) {
        let dx = x - (BOARD_X + BOARD_W / 2);
        let dy = y - (BOARD_Y + BOARD_H / 2);
        let (dx, dy) = if dx.abs() > dy.abs() {
            (dx.signum() as i8, 0)
        } else {
            (0, dy.signum() as i8)
        };
        if dx == 0 && dy == 0 {
            return;
        }
        // Reversing into your own neck is the one turn that is never wanted.
        if dx == -self.dx && dy == -self.dy {
            return;
        }
        self.pending = Some((dx, dy));
    }

    pub fn update(&mut self, now_ms: u32, bubbles: &mut Bubbles) {
        if self.phase != Phase::Run || now_ms < self.next_step_ms {
            return;
        }
        self.next_step_ms = now_ms + self.step_ms;
        if let Some((dx, dy)) = self.pending.take() {
            self.dx = dx;
            self.dy = dy;
        }

        let (hx, hy) = self.body[self.head];
        let (nx, ny) = (hx + self.dx, hy + self.dy);
        // Walls kill - the phone's did, and a wrapping worm is a different game.
        if nx < 0 || ny < 0 || nx >= COLS as i8 || ny >= ROWS as i8 {
            self.die(bubbles, now_ms, hx, hy);
            return;
        }
        // The tail cell is about to move out from under the head, so it does not
        // count as a collision unless the worm is about to grow into it.
        let eating = (nx, ny) == self.food;
        let ignore_tail = if eating { 0 } else { 1 };
        for index in 0..self.len.saturating_sub(ignore_tail) {
            if self.cell(index) == (nx, ny) {
                self.die(bubbles, now_ms, hx, hy);
                return;
            }
        }

        self.head = (self.head + MAX_LEN - 1) % MAX_LEN;
        self.body[self.head] = (nx, ny);
        if eating {
            self.len = (self.len + 1).min(MAX_LEN);
            self.score += 1;
            self.best = self.best.max(self.score);
            self.step_ms = self.step_ms.saturating_sub(STEP_GAIN_MS).max(STEP_MIN_MS);
            let (px, py) = pixel(nx, ny);
            bubbles.spawn(px, py, now_ms, Some(muted(C_FOOD)), Some(90), true);
            self.place_food();
        }
    }

    fn die(&mut self, bubbles: &mut Bubbles, now_ms: u32, hx: i8, hy: i8) {
        self.phase = Phase::Over;
        let (px, py) = pixel(hx, hy);
        bubbles.spawn(px, py, now_ms, Some(muted(C_BODY)), None, true);
    }

    /// Cell `index` back from the head, 0 being the head itself.
    fn cell(&self, index: usize) -> (i8, i8) {
        self.body[(self.head + index) % MAX_LEN]
    }

    /// Somewhere the worm is not. The board always has room: the worm would have
    /// to fill it entirely, which is the win condition and not reachable here.
    fn place_food(&mut self) {
        for _ in 0..200 {
            self.seed ^= self.seed << 13;
            self.seed ^= self.seed >> 17;
            self.seed ^= self.seed << 5;
            let x = (self.seed % COLS as u32) as i8;
            let y = ((self.seed / 64) % ROWS as u32) as i8;
            if (0..self.len).all(|index| self.cell(index) != (x, y)) {
                self.food = (x, y);
                return;
            }
        }
    }

    pub fn draw(&self, scene: &mut Scene, alpha: u8) {
        let mut line = TextBuf::new();
        use core::fmt::Write as _;
        let _ = write!(line, "{}  BEST {}", self.score, self.best);
        scene.label(
            W as i32 / 2,
            62,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            line.as_str(),
        );

        scene.pill(
            BOARD_X - 4,
            BOARD_Y - 4,
            BOARD_X + BOARD_W + 4,
            BOARD_Y + BOARD_H + 4,
            10,
            C_EDGE,
            alpha,
        );
        scene.pill(
            BOARD_X,
            BOARD_Y,
            BOARD_X + BOARD_W,
            BOARD_Y + BOARD_H,
            8,
            C_BOARD,
            alpha,
        );

        let (fx, fy) = pixel(self.food.0, self.food.1);
        scene.disc(fx, fy, CELL / 2 - 5, C_FOOD, alpha);

        // Merge collinear runs, so a straight length is one bar.
        let mut index = 0;
        while index < self.len {
            let (sx, sy) = self.cell(index);
            let mut end = index;
            while end + 1 < self.len {
                let (ax, ay) = self.cell(end);
                let (bx, by) = self.cell(end + 1);
                let down = ax == bx && bx == sx && (by - ay).abs() == 1;
                let across = ay == by && by == sy && (bx - ax).abs() == 1;
                if !(down || across) {
                    break;
                }
                end += 1;
            }
            let (ex, ey) = self.cell(end);
            let (x0, y0) = pixel(sx.min(ex), sy.min(ey));
            let (x1, y1) = pixel(sx.max(ex), sy.max(ey));
            let inset = 3;
            scene.pill(
                x0 - CELL / 2 + inset,
                y0 - CELL / 2 + inset,
                x1 + CELL / 2 - inset,
                y1 + CELL / 2 - inset,
                CELL / 2 - inset,
                C_BODY,
                alpha,
            );
            index = end + 1;
        }

        let (hx, hy) = self.cell(0);
        let (px, py) = pixel(hx, hy);
        scene.disc(px, py, CELL / 2 - 3, C_HEAD, alpha);

        if self.phase != Phase::Run {
            scene.pill(
                80,
                H as i32 / 2 - 34,
                W as i32 - 80,
                H as i32 / 2 + 26,
                30,
                rgb(8, 12, 10),
                alpha,
            );
            scene.label(
                W as i32 / 2,
                H as i32 / 2 + 4,
                FontId::Body,
                if self.phase == Phase::Over { C_FOOD } else { INK },
                alpha,
                Align::Center,
                if self.phase == Phase::Over {
                    "TAP TO RETRY"
                } else {
                    "TAP TO START"
                },
            );
        }
    }
}

/// Centre of a cell, in pixels.
fn pixel(cx: i8, cy: i8) -> (i32, i32) {
    (
        BOARD_X + cx as i32 * CELL + CELL / 2,
        BOARD_Y + cy as i32 * CELL + CELL / 2,
    )
}
