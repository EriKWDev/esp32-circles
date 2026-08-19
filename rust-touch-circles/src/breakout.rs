//! Breakout, driving the same circles the demo does.
//!
//! Three authored layouts and six palettes, indexed independently: the layout
//! repeats every third level while the colours carry on changing, so level four
//! is a familiar wall in unfamiliar colours rather than a straight repeat.

use crate::bubbles::Bubbles;
use crate::font::FontId;
use crate::gfx::{Align, H, Scene, W, muted, rgb};

const COLS: usize = 8;
const ROWS: usize = 5;

const MARGIN: i32 = 16;
const CELL_W: i32 = (W as i32 - 2 * MARGIN) / COLS as i32;
const CELL_H: i32 = 26;
const WALL_TOP: i32 = 104;
/// Inset of the drawn brick within its cell, which is where the grid gets its
/// gaps without any second set of numbers.
const BRICK_PAD: i32 = 3;

const PADDLE_Y: i32 = 436;
const PADDLE_W: i32 = 104;
const PADDLE_H: i32 = 14;
const BALL_R: i32 = 8;

const Q: i32 = 8;
const SPEED: i32 = 250 * Q;
const SPEED_MAX: i32 = 520 * Q;
/// Gained per brick, so a long rally accelerates.
const SPEED_GAIN: i32 = 2 * Q;
const SERVE_PAUSE_MS: u32 = 700;
const LIVES: u8 = 3;

pub struct Palette {
    pub bg: u16,
    /// One per layout digit, so a level's own numbers pick out its colours.
    pub bricks: [u16; 4],
    pub paddle: u16,
    pub ball: u16,
}

/// Deliberately low-key backgrounds: on an OLED a dark ground costs almost
/// nothing to light and lets the bricks be the bright thing.
const PALETTES: [Palette; 6] = [
    Palette {
        bg: rgb(6, 10, 20),
        bricks: [
            rgb(88, 190, 255),
            rgb(120, 150, 255),
            rgb(160, 120, 255),
            rgb(210, 110, 240),
        ],
        paddle: rgb(120, 220, 255),
        ball: rgb(240, 248, 255),
    },
    Palette {
        bg: rgb(16, 8, 6),
        bricks: [
            rgb(255, 190, 70),
            rgb(255, 140, 60),
            rgb(240, 90, 70),
            rgb(200, 60, 110),
        ],
        paddle: rgb(255, 200, 110),
        ball: rgb(255, 246, 230),
    },
    Palette {
        bg: rgb(4, 16, 12),
        bricks: [
            rgb(120, 240, 170),
            rgb(70, 220, 200),
            rgb(60, 190, 230),
            rgb(180, 240, 120),
        ],
        paddle: rgb(140, 245, 190),
        ball: rgb(236, 255, 246),
    },
    Palette {
        bg: rgb(14, 6, 18),
        bricks: [
            rgb(255, 120, 200),
            rgb(220, 110, 255),
            rgb(150, 120, 255),
            rgb(255, 160, 230),
        ],
        paddle: rgb(255, 150, 220),
        ball: rgb(255, 240, 250),
    },
    Palette {
        bg: rgb(18, 14, 4),
        bricks: [
            rgb(240, 230, 120),
            rgb(210, 240, 90),
            rgb(160, 220, 80),
            rgb(250, 200, 90),
        ],
        paddle: rgb(240, 236, 140),
        ball: rgb(255, 253, 236),
    },
    Palette {
        bg: rgb(8, 12, 16),
        bricks: [
            rgb(200, 214, 230),
            rgb(150, 172, 196),
            rgb(110, 190, 210),
            rgb(230, 240, 250),
        ],
        paddle: rgb(214, 226, 240),
        ball: rgb(255, 255, 255),
    },
];

/// A digit picks a brick colour; a dot is a hole. Eight columns by five rows.
const LAYOUTS: [[&str; ROWS]; 3] = [
    // Solid bands, to learn the angles on.
    ["00000000", "11111111", "22222222", "33333333", "..2222.."],
    // A chevron, which leaves lanes to thread the ball through.
    ["3......3", "23....32", "12.33.21", "0122210.", "..0110.."],
    // Checker and a spine, so the last bricks are awkward.
    ["0.1.1.0.", ".2.33.2.", "3.0..0.3", ".11..11.", "..3223.."],
];

pub struct Breakout {
    /// 0 is empty, otherwise the brick's colour index plus one.
    cells: [[u8; COLS]; ROWS],
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
    paddle_x: i32,
    pub level: u32,
    lives: u8,
    score: u32,
    serve_at_ms: u32,
    /// Cleared or out of lives, waiting to hand over to the next state.
    settling: bool,
}

impl Breakout {
    pub const fn new() -> Self {
        Self {
            cells: [[0; COLS]; ROWS],
            x: (W as i32 / 2) * Q,
            y: (PADDLE_Y - 40) * Q,
            vx: 0,
            vy: 0,
            paddle_x: W as i32 / 2,
            level: 0,
            lives: LIVES,
            score: 0,
            serve_at_ms: 0,
            settling: false,
        }
    }

    pub fn palette(&self) -> &'static Palette {
        &PALETTES[self.level as usize % PALETTES.len()]
    }

    pub fn restart(&mut self, now_ms: u32) {
        *self = Self::new();
        self.load_level(now_ms);
    }

    fn load_level(&mut self, now_ms: u32) {
        let layout = &LAYOUTS[self.level as usize % LAYOUTS.len()];
        for (row, line) in layout.iter().enumerate() {
            for (col, ch) in line.bytes().take(COLS).enumerate() {
                self.cells[row][col] = match ch {
                    b'0'..=b'3' => ch - b'0' + 1,
                    _ => 0,
                };
            }
        }
        self.park(now_ms);
    }

    /// Ball back on the paddle, held for a moment.
    fn park(&mut self, now_ms: u32) {
        self.x = self.paddle_x * Q;
        self.y = (PADDLE_Y - BALL_R - 2) * Q;
        self.vx = 0;
        self.vy = 0;
        self.serve_at_ms = now_ms + SERVE_PAUSE_MS;
        self.settling = false;
    }

    pub fn touch(&mut self, x: i32) {
        self.paddle_x = x.clamp(PADDLE_W / 2, W as i32 - PADDLE_W / 2);
    }

    pub fn update(&mut self, dt_ms: u32, now_ms: u32, bubbles: &mut Bubbles) {
        if now_ms < self.serve_at_ms {
            // Waiting to serve: the ball rides the paddle, so the first shot is
            // aimed rather than random.
            if self.vx == 0 {
                self.x = self.paddle_x * Q;
            }
            return;
        }
        if self.vx == 0 && self.vy == 0 {
            self.vx = if now_ms % 2 == 0 {
                SPEED / 2
            } else {
                -SPEED / 2
            };
            self.vy = -SPEED;
        }

        let dt = dt_ms.min(40) as i32;
        self.x += self.vx * dt / 1000;
        self.y += self.vy * dt / 1000;

        // Walls: sides and top.
        let (left, right) = (BALL_R * Q, (W as i32 - BALL_R) * Q);
        if self.x <= left && self.vx < 0 || self.x >= right && self.vx > 0 {
            self.x = self.x.clamp(left, right);
            self.vx = -self.vx;
            bubbles.spawn(
                self.x / Q,
                self.y / Q,
                now_ms,
                Some(muted(self.palette().ball)),
                Some(64),
                true,
            );
        }
        if self.y <= BALL_R * Q && self.vy < 0 {
            self.y = BALL_R * Q;
            self.vy = -self.vy;
            bubbles.spawn(
                self.x / Q,
                self.y / Q,
                now_ms,
                Some(muted(self.palette().ball)),
                Some(64),
                true,
            );
        }

        self.hit_brick(now_ms, bubbles);
        self.hit_paddle(now_ms, bubbles);

        // Lost.
        if self.y > (H as i32 + BALL_R) * Q && !self.settling {
            self.lives = self.lives.saturating_sub(1);
            bubbles.spawn(
                self.x.clamp(0, (W as i32) * Q) / Q,
                H as i32 - 10,
                now_ms,
                Some(muted(rgb(220, 60, 60))),
                Some(220),
                true,
            );
            if self.lives == 0 {
                self.settling = true;
                self.serve_at_ms = now_ms + SERVE_PAUSE_MS * 2;
            } else {
                self.park(now_ms);
            }
        }

        // Out of lives, and the pause has run: start again from the first level.
        if self.settling && self.lives == 0 && now_ms >= self.serve_at_ms {
            let score = self.score;
            self.restart(now_ms);
            self.score = score;
        }
    }

    fn hit_brick(&mut self, now_ms: u32, bubbles: &mut Bubbles) {
        let (bx, by) = (self.x / Q, self.y / Q);
        let row = (by - WALL_TOP) / CELL_H;
        let col = (bx - MARGIN) / CELL_W;
        if row < 0 || col < 0 || row >= ROWS as i32 || col >= COLS as i32 {
            return;
        }
        let (row, col) = (row as usize, col as usize);
        let cell = self.cells[row][col];
        if cell == 0 {
            return;
        }
        self.cells[row][col] = 0;
        self.score += 10;

        // Reflect on whichever axis the ball entered by, judged from how far into
        // the cell it is: a shallow entry through the side must not read as a hit
        // from below.
        let cell_cx = MARGIN + col as i32 * CELL_W + CELL_W / 2;
        let cell_cy = WALL_TOP + row as i32 * CELL_H + CELL_H / 2;
        let overlap_x = (CELL_W / 2 + BALL_R) - (bx - cell_cx).abs();
        let overlap_y = (CELL_H / 2 + BALL_R) - (by - cell_cy).abs();
        if overlap_y < overlap_x {
            self.vy = -self.vy;
        } else {
            self.vx = -self.vx;
        }
        // A brick's own colour, so the wall dissolves into its own palette.
        let color = self.palette().bricks[(cell - 1) as usize % 4];
        bubbles.spawn(
            cell_cx,
            cell_cy,
            now_ms,
            Some(muted(color)),
            Some(130),
            true,
        );

        let speed = self.vy.abs() + SPEED_GAIN;
        self.vy = self.vy.signum() * speed.min(SPEED_MAX);

        if self.cleared() {
            bubbles.spawn(cell_cx, cell_cy, now_ms, Some(muted(color)), None, true);
            self.level += 1;
            self.load_level(now_ms);
            self.serve_at_ms = now_ms + SERVE_PAUSE_MS * 2;
        }
    }

    fn hit_paddle(&mut self, now_ms: u32, bubbles: &mut Bubbles) {
        if self.vy <= 0 {
            return;
        }
        let face = (PADDLE_Y - BALL_R) * Q;
        if self.y < face || self.y > (PADDLE_Y + PADDLE_H) * Q {
            return;
        }
        if (self.x / Q - self.paddle_x).abs() > PADDLE_W / 2 + BALL_R {
            return;
        }
        self.y = face;
        self.vy = -self.vy;
        // Where it lands steers the return, which is what makes the paddle a
        // control rather than a wall.
        let offset = self.x / Q - self.paddle_x;
        self.vx += offset * (SPEED / 90);
        self.vx = self.vx.clamp(-SPEED_MAX, SPEED_MAX);
        bubbles.spawn(
            self.x / Q,
            PADDLE_Y,
            now_ms,
            Some(muted(self.palette().paddle)),
            Some(160),
            true,
        );
    }

    fn cleared(&self) -> bool {
        !self.cells.iter().flatten().any(|cell| *cell != 0)
    }

    pub fn draw(&self, scene: &mut Scene, alpha: u8) {
        let palette = self.palette();

        for (row, line) in self.cells.iter().enumerate() {
            for (col, cell) in line.iter().enumerate() {
                if *cell == 0 {
                    continue;
                }
                let x0 = MARGIN + col as i32 * CELL_W + BRICK_PAD;
                let y0 = WALL_TOP + row as i32 * CELL_H + BRICK_PAD;
                scene.pill(
                    x0,
                    y0,
                    x0 + CELL_W - 2 * BRICK_PAD,
                    y0 + CELL_H - 2 * BRICK_PAD,
                    5,
                    palette.bricks[(*cell - 1) as usize % 4],
                    alpha,
                );
            }
        }

        scene.pill(
            self.paddle_x - PADDLE_W / 2,
            PADDLE_Y,
            self.paddle_x + PADDLE_W / 2,
            PADDLE_Y + PADDLE_H,
            PADDLE_H / 2,
            palette.paddle,
            alpha,
        );
        scene.disc(self.x / Q, self.y / Q, BALL_R, palette.ball, alpha);

        // Level, score and lives on one line, above the wall.
        use core::fmt::Write as _;
        let mut line = crate::gfx::TextBuf::new();
        let _ = write!(
            line,
            "LEVEL {} \u{b7} {} \u{b7} {}",
            self.level + 1,
            self.score,
            self.lives
        );
        scene.label(
            W as i32 / 2,
            76,
            FontId::Caption,
            palette.bricks[0],
            alpha,
            Align::Center,
            line.as_str(),
        );
    }
}
