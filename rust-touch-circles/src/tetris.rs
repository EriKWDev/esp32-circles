//! Tetris.
//!
//! Pieces are 4x4 bitmasks, one per rotation, rather than a rotation routine. All
//! twenty-eight are written out: it is more text but there is nothing left to get
//! wrong at runtime, and the awkward cases - the I piece's two distinct states, S
//! and Z being reflections rather than rotations - are simply looked up.
//!
//! Settled cells are drawn as merged horizontal runs of one colour. A full board
//! is two hundred cells and the scene holds fewer primitives than that, so a cell
//! per block would quietly stop drawing part of the stack; a row of one colour is
//! one rounded bar however wide it is.
//!
//! Controls are buttons, not gestures. The worm taught that lesson: a gesture
//! gives no clue what it thinks you meant, and a stack under time pressure is the
//! worst place to find out.

use crate::bubbles::Bubbles;
use crate::font::FontId;
use crate::gfx::{Align, Scene, TextBuf, muted, rgb};

const COLS: usize = 10;
const ROWS: usize = 20;
const CELL: i32 = 18;
const BOARD_X: i32 = 16;
const BOARD_Y: i32 = 80;
const BOARD_W: i32 = CELL * COLS as i32;
const BOARD_H: i32 = CELL * ROWS as i32;

/// Right-hand column: the next piece, the numbers, and the controls.
const PANEL_X: i32 = 210;
pub const ROTATE: (i32, i32, i32, i32) = (PANEL_X, 280, 464, 340);
pub const LEFT: (i32, i32, i32, i32) = (PANEL_X, 348, 334, 408);
pub const RIGHT: (i32, i32, i32, i32) = (340, 348, 464, 408);
pub const DROP: (i32, i32, i32, i32) = (PANEL_X, 416, 464, 472);

/// Gravity at level zero, and the floor it approaches. One row per interval.
const FALL_MS: u32 = 700;
const FALL_MIN_MS: u32 = 90;
/// Taken off the interval per level.
const FALL_STEP_MS: u32 = 55;
const LINES_PER_LEVEL: u32 = 10;
/// How long a completed row is shown lit before the stack collapses onto it.
const FLASH_MS: u32 = 170;
/// A held direction repeats, after a pause long enough that a tap moves one cell.
const REPEAT_DELAY_MS: u32 = 230;
const REPEAT_MS: u32 = 110;

const INK: u16 = rgb(238, 245, 250);
const DIM: u16 = rgb(140, 152, 166);
const WELL: u16 = rgb(12, 14, 20);
const EDGE: u16 = rgb(38, 44, 56);
const PLATE: u16 = rgb(26, 30, 38);
const FLASH: u16 = rgb(250, 250, 250);
pub const ACCENT: u16 = rgb(120, 210, 235);

/// The seven pieces, four rotations each, as bits of a 4x4 box read row by row.
const SHAPES: [[u16; 4]; 7] = [
    // I
    [0x00F0, 0x4444, 0x00F0, 0x4444],
    // O
    [0x0660, 0x0660, 0x0660, 0x0660],
    // T
    [0x0072, 0x0262, 0x0270, 0x0232],
    // S
    [0x0036, 0x0462, 0x0360, 0x0231],
    // Z
    [0x0063, 0x0264, 0x0630, 0x0132],
    // J
    [0x0071, 0x0226, 0x0470, 0x0322],
    // L
    [0x0074, 0x0622, 0x0170, 0x0223],
];

const COLORS: [u16; 7] = [
    rgb(80, 220, 240),  // I
    rgb(245, 215, 80),  // O
    rgb(180, 110, 240), // T
    rgb(110, 225, 130), // S
    rgb(240, 100, 100), // Z
    rgb(90, 140, 245),  // J
    rgb(245, 160, 70),  // L
];

/// Points for one, two, three and four rows, before the level multiplier.
const LINE_SCORE: [u32; 5] = [0, 40, 100, 300, 1200];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Ready,
    Falling,
    /// Rows are complete and lit; the stack drops when this expires.
    Clearing,
    Over,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Rotate,
    Left,
    Right,
    Drop,
}

pub struct Tetris {
    /// Settled cells, one colour index plus one, zero for empty.
    well: [[u8; COLS]; ROWS],
    piece: usize,
    next: usize,
    rotation: usize,
    /// Top-left of the piece's 4x4 box, in cells. Signed: the box overhangs.
    px: i32,
    py: i32,
    phase: Phase,
    fall_at_ms: u32,
    score: u32,
    lines: u32,
    level: u32,
    pub best: u32,
    /// Rows waiting to be cleared, and when the flash ends.
    full: [bool; ROWS],
    clear_at_ms: u32,
    /// The button under the finger, and when it may act again.
    held: Option<Button>,
    repeat_at_ms: u32,
    /// Seven-bag randomiser: which of the seven are still to come.
    bag: [usize; 7],
    bag_left: usize,
    seed: u32,
}

impl Tetris {
    pub const fn new() -> Self {
        Self {
            well: [[0; COLS]; ROWS],
            piece: 0,
            next: 1,
            rotation: 0,
            px: 3,
            py: 0,
            phase: Phase::Ready,
            fall_at_ms: 0,
            score: 0,
            lines: 0,
            level: 0,
            best: 0,
            full: [false; ROWS],
            clear_at_ms: 0,
            held: None,
            repeat_at_ms: 0,
            bag: [0, 1, 2, 3, 4, 5, 6],
            bag_left: 0,
            seed: 0x2f6e_2b1c,
        }
    }

    pub fn restart(&mut self, now_ms: u32) {
        let best = self.best;
        *self = Self::new();
        self.best = best;
        self.seed ^= now_ms | 1;
        self.next = self.take_from_bag();
        self.spawn(now_ms);
        self.phase = Phase::Ready;
    }

    pub fn tap(&mut self, now_ms: u32) {
        match self.phase {
            Phase::Ready => {
                self.phase = Phase::Falling;
                self.fall_at_ms = now_ms + self.interval();
            }
            Phase::Over => self.restart(now_ms),
            _ => {}
        }
    }

    fn interval(&self) -> u32 {
        FALL_MS
            .saturating_sub(self.level * FALL_STEP_MS)
            .max(FALL_MIN_MS)
    }

    /// One piece from the bag, refilled and shuffled when empty. A bag means the
    /// same piece cannot come four times running, which pure chance allows and
    /// which feels broken rather than unlucky.
    fn take_from_bag(&mut self) -> usize {
        if self.bag_left == 0 {
            self.bag = [0, 1, 2, 3, 4, 5, 6];
            self.bag_left = 7;
            for index in (1..7).rev() {
                self.seed ^= self.seed << 13;
                self.seed ^= self.seed >> 17;
                self.seed ^= self.seed << 5;
                let swap = (self.seed % (index as u32 + 1)) as usize;
                self.bag.swap(index, swap);
            }
        }
        self.bag_left -= 1;
        self.bag[self.bag_left]
    }

    fn spawn(&mut self, now_ms: u32) {
        self.piece = self.next;
        self.next = self.take_from_bag();
        self.rotation = 0;
        self.px = 3;
        self.py = 0;
        self.fall_at_ms = now_ms + self.interval();
        // No room for the new piece is the end: the stack has reached the top.
        if !self.fits(self.px, self.py, self.rotation) {
            self.phase = Phase::Over;
            self.best = self.best.max(self.score);
        }
    }

    /// Whether the piece's cells are all inside the well and on empty squares.
    fn fits(&self, px: i32, py: i32, rotation: usize) -> bool {
        let shape = SHAPES[self.piece][rotation & 3];
        for bit in 0..16 {
            if shape & (1 << bit) == 0 {
                continue;
            }
            let x = px + (bit % 4) as i32;
            let y = py + (bit / 4) as i32;
            if x < 0 || x >= COLS as i32 || y >= ROWS as i32 {
                return false;
            }
            // Above the well is allowed while a piece is still entering.
            if y >= 0 && self.well[y as usize][x as usize] != 0 {
                return false;
            }
        }
        true
    }

    pub fn press(&mut self, x: i32, y: i32, now_ms: u32) {
        let Some(button) = button_at(x, y) else {
            self.tap(now_ms);
            return;
        };
        if self.phase != Phase::Falling {
            self.tap(now_ms);
            return;
        }
        self.held = Some(button);
        self.repeat_at_ms = now_ms + REPEAT_DELAY_MS;
        self.act(button, now_ms);
    }

    /// A finger that slides off its button stops repeating; one that slides onto
    /// another takes that one over.
    pub fn drag(&mut self, x: i32, y: i32, now_ms: u32) {
        match button_at(x, y) {
            Some(button) if Some(button) != self.held => {
                self.held = Some(button);
                self.repeat_at_ms = now_ms + REPEAT_DELAY_MS;
                self.act(button, now_ms);
            }
            Some(_) => {}
            None => self.held = None,
        }
    }

    pub fn release(&mut self) {
        self.held = None;
    }

    fn act(&mut self, button: Button, now_ms: u32) {
        if self.phase != Phase::Falling {
            return;
        }
        match button {
            Button::Left => {
                if self.fits(self.px - 1, self.py, self.rotation) {
                    self.px -= 1;
                }
            }
            Button::Right => {
                if self.fits(self.px + 1, self.py, self.rotation) {
                    self.px += 1;
                }
            }
            Button::Rotate => {
                let turned = (self.rotation + 1) & 3;
                // Nudge sideways if the turn only fails against a wall or a
                // neighbour - a rotation that needs one column is the one players
                // expect to work.
                for shift in [0, -1, 1, -2, 2] {
                    if self.fits(self.px + shift, self.py, turned) {
                        self.px += shift;
                        self.rotation = turned;
                        break;
                    }
                }
            }
            Button::Drop => {
                // All the way down, and locked at once: the drop is the commitment.
                let mut fell = 0;
                while self.fits(self.px, self.py + 1, self.rotation) {
                    self.py += 1;
                    fell += 1;
                }
                self.score += fell as u32;
                self.fall_at_ms = now_ms;
            }
        }
    }

    pub fn update(&mut self, now_ms: u32, bubbles: &mut Bubbles) {
        // A held direction repeats so a piece can cross the well without a tap per
        // column.
        if let Some(button) = self.held
            && matches!(button, Button::Left | Button::Right)
            && now_ms.wrapping_sub(self.repeat_at_ms) < u32::MAX / 2
        {
            self.repeat_at_ms = now_ms + REPEAT_MS;
            self.act(button, now_ms);
        }

        match self.phase {
            Phase::Clearing => {
                if now_ms.wrapping_sub(self.clear_at_ms) < u32::MAX / 2 {
                    self.collapse();
                    self.spawn(now_ms);
                    if self.phase != Phase::Over {
                        self.phase = Phase::Falling;
                    }
                }
            }
            Phase::Falling => {
                if now_ms.wrapping_sub(self.fall_at_ms) > u32::MAX / 2 {
                    return;
                }
                if self.fits(self.px, self.py + 1, self.rotation) {
                    self.py += 1;
                    self.fall_at_ms = now_ms + self.interval();
                } else {
                    self.lock(now_ms, bubbles);
                }
            }
            _ => {}
        }
    }

    fn lock(&mut self, now_ms: u32, bubbles: &mut Bubbles) {
        let shape = SHAPES[self.piece][self.rotation & 3];
        for bit in 0..16 {
            if shape & (1 << bit) == 0 {
                continue;
            }
            let x = self.px + (bit % 4) as i32;
            let y = self.py + (bit / 4) as i32;
            if y >= 0 && y < ROWS as i32 && x >= 0 && x < COLS as i32 {
                self.well[y as usize][x as usize] = self.piece as u8 + 1;
            }
        }

        let mut count = 0;
        self.full = [false; ROWS];
        for row in 0..ROWS {
            if self.well[row].iter().all(|cell| *cell != 0) {
                self.full[row] = true;
                count += 1;
            }
        }
        if count == 0 {
            self.spawn(now_ms);
            return;
        }

        self.lines += count as u32;
        self.level = self.lines / LINES_PER_LEVEL;
        self.score += LINE_SCORE[count.min(4)] * (self.level + 1);
        self.best = self.best.max(self.score);
        // One circle per row, and a big one for a tetris.
        for row in 0..ROWS {
            if self.full[row] {
                bubbles.spawn(
                    BOARD_X + BOARD_W / 2,
                    BOARD_Y + row as i32 * CELL + CELL / 2,
                    now_ms,
                    Some(if count == 4 { ACCENT } else { muted(ACCENT) }),
                    Some(if count == 4 { 200 } else { 90 }),
                    true,
                );
            }
        }
        self.phase = Phase::Clearing;
        self.clear_at_ms = now_ms + FLASH_MS;
    }

    /// Drop everything above each cleared row down onto it.
    fn collapse(&mut self) {
        let mut write = ROWS - 1;
        for read in (0..ROWS).rev() {
            if self.full[read] {
                continue;
            }
            self.well[write] = self.well[read];
            write = write.saturating_sub(1);
        }
        // Whatever is left at the top is new empty space.
        for row in 0..=write {
            self.well[row] = [0; COLS];
        }
        self.full = [false; ROWS];
    }

    /// Where the piece would land, for the guide at the bottom of the well.
    fn shadow_y(&self) -> i32 {
        let mut y = self.py;
        while self.fits(self.px, y + 1, self.rotation) {
            y += 1;
        }
        y
    }

    pub fn draw(&self, scene: &mut Scene, alpha: u8) {
        scene.pill(
            BOARD_X - 4,
            BOARD_Y - 4,
            BOARD_X + BOARD_W + 4,
            BOARD_Y + BOARD_H + 4,
            8,
            EDGE,
            alpha,
        );
        scene.pill(
            BOARD_X,
            BOARD_Y,
            BOARD_X + BOARD_W,
            BOARD_Y + BOARD_H,
            6,
            WELL,
            alpha,
        );

        // Settled cells, as runs of one colour: a full row costs one primitive
        // instead of ten, which is what keeps a tall stack inside the budget.
        for row in 0..ROWS {
            let lit = self.full[row];
            let mut col = 0;
            while col < COLS {
                let cell = self.well[row][col];
                if cell == 0 {
                    col += 1;
                    continue;
                }
                let mut end = col;
                while end + 1 < COLS && self.well[row][end + 1] == cell {
                    end += 1;
                }
                let color = if lit {
                    FLASH
                } else {
                    COLORS[(cell - 1) as usize % 7]
                };
                scene.pill(
                    BOARD_X + col as i32 * CELL + 1,
                    BOARD_Y + row as i32 * CELL + 1,
                    BOARD_X + (end + 1) as i32 * CELL - 1,
                    BOARD_Y + (row + 1) as i32 * CELL - 1,
                    4,
                    color,
                    alpha,
                );
                col = end + 1;
            }
        }

        if self.phase == Phase::Falling || self.phase == Phase::Ready {
            let shape = SHAPES[self.piece][self.rotation & 3];
            let color = COLORS[self.piece % 7];
            // The landing guide first, so the piece itself draws over it.
            let shadow = self.shadow_y();
            if shadow != self.py {
                self.draw_shape(scene, shape, self.px, shadow, muted(WELL), alpha, true);
            }
            self.draw_shape(scene, shape, self.px, self.py, color, alpha, false);
        }

        // Next piece, in its own box.
        scene.pill(PANEL_X, 84, 464, 176, 10, PLATE, alpha);
        scene.label(
            PANEL_X + 14,
            108,
            FontId::Micro,
            DIM,
            alpha,
            Align::Left,
            "NEXT",
        );
        let preview = SHAPES[self.next][0];
        for bit in 0..16 {
            if preview & (1 << bit) == 0 {
                continue;
            }
            let x = 300 + (bit % 4) as i32 * 16;
            let y = 122 + (bit / 4) as i32 * 16;
            scene.pill(x, y, x + 14, y + 14, 3, COLORS[self.next % 7], alpha);
        }

        let mut line = TextBuf::new();
        use core::fmt::Write as _;
        let _ = write!(line, "{}", self.score);
        scene.label(
            464,
            212,
            FontId::Body,
            INK,
            alpha,
            Align::Right,
            line.as_str(),
        );
        let mut line = TextBuf::new();
        let _ = write!(line, "BEST {}", self.best);
        scene.label(
            464,
            238,
            FontId::Micro,
            DIM,
            alpha,
            Align::Right,
            line.as_str(),
        );
        let mut line = TextBuf::new();
        let _ = write!(line, "LEVEL {}  LINES {}", self.level, self.lines);
        scene.label(
            464,
            262,
            FontId::Micro,
            DIM,
            alpha,
            Align::Right,
            line.as_str(),
        );

        for (rect, caption) in [
            (ROTATE, "ROTATE"),
            (LEFT, "<"),
            (RIGHT, ">"),
            (DROP, "DROP"),
        ] {
            let (x0, y0, x1, y1) = rect;
            let live = self.held == button_at((x0 + x1) / 2, (y0 + y1) / 2);
            scene.pill(
                x0,
                y0,
                x1,
                y1,
                (y1 - y0) / 2,
                if live { muted(ACCENT) } else { PLATE },
                alpha,
            );
            scene.label(
                (x0 + x1) / 2,
                (y0 + y1) / 2 + 10,
                FontId::Caption,
                if live { rgb(8, 12, 16) } else { ACCENT },
                alpha,
                Align::Center,
                caption,
            );
        }

        if self.phase == Phase::Ready || self.phase == Phase::Over {
            let over = self.phase == Phase::Over;
            scene.pill(24, 220, 200, 280, 30, rgb(8, 10, 14), alpha);
            scene.label(
                112,
                258,
                FontId::Caption,
                if over { rgb(245, 120, 100) } else { INK },
                alpha,
                Align::Center,
                if over { "TAP TO RETRY" } else { "TAP TO START" },
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_shape(
        &self,
        scene: &mut Scene,
        shape: u16,
        px: i32,
        py: i32,
        color: u16,
        alpha: u8,
        outline: bool,
    ) {
        for bit in 0..16 {
            if shape & (1 << bit) == 0 {
                continue;
            }
            let x = px + (bit % 4) as i32;
            let y = py + (bit / 4) as i32;
            if y < 0 || y >= ROWS as i32 || x < 0 || x >= COLS as i32 {
                continue;
            }
            let x0 = BOARD_X + x * CELL + 1;
            let y0 = BOARD_Y + y * CELL + 1;
            if outline {
                // The guide is a ring so it cannot be mistaken for a settled cell.
                scene.ring(
                    x0 + CELL / 2 - 1,
                    y0 + CELL / 2 - 1,
                    CELL / 2 - 2,
                    CELL / 2 - 4,
                    EDGE,
                    alpha,
                );
            } else {
                scene.pill(x0, y0, x0 + CELL - 2, y0 + CELL - 2, 4, color, alpha);
            }
        }
    }
}

pub fn button_at(x: i32, y: i32) -> Option<Button> {
    for (rect, button) in [
        (ROTATE, Button::Rotate),
        (LEFT, Button::Left),
        (RIGHT, Button::Right),
        (DROP, Button::Drop),
    ] {
        let (x0, y0, x1, y1) = rect;
        if x >= x0 && x <= x1 && y >= y0 && y <= y1 {
            return Some(button);
        }
    }
    None
}
