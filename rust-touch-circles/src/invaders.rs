//! Space invaders. Drag to move; the gun fires by itself.
//!
//! Auto-fire because the panel reports one contact at a time: a fire button would
//! mean letting go of the ship to shoot. The cadence is the difficulty knob
//! instead.

use crate::bubbles::Bubbles;
use crate::font::FontId;
use crate::gfx::{Align, H, Scene, TextBuf, W, muted, rgb};

const COLS: usize = 7;
const ROWS: usize = 4;
const MAX_SHOTS: usize = 3;
const MAX_BOMBS: usize = 3;

const CELL_W: i32 = 54;
const CELL_H: i32 = 40;
const INVADER_R: i32 = 15;
const FLEET_TOP: i32 = 112;

const SHIP_Y: i32 = 438;
const SHIP_R: i32 = 16;
const SHOT_R: i32 = 4;
const BOMB_R: i32 = 5;

const Q: i32 = 8;
const SHOT_SPEED: i32 = 460 * Q;
const BOMB_SPEED: i32 = 190 * Q;
/// Sideways fleet speed for wave one; each wave adds a share of it.
const FLEET_SPEED: i32 = 34 * Q;
const FLEET_DROP: i32 = 16;
/// Pixels a second for that drop - fast enough to feel like a step, slow enough
/// to be a movement rather than a jump.
const DROP_SPEED: i32 = 72;
const FIRE_EVERY_MS: u32 = 420;
const BOMB_EVERY_MS: u32 = 900;
const LIVES: u8 = 3;

/// Four bunkers of BUNKER_W by BUNKER_H blocks. Deliberately coarse: each block
/// is a primitive, and the fleet above already accounts for most of the frame.
const BUNKERS: usize = 4;
const BUNKER_W: usize = 4;
const BUNKER_H: usize = 2;
const BLOCK: i32 = 22;
const BUNKER_TOP: i32 = SHIP_Y - 78;

/// Row colours, per wave. Six sets, so the wave keeps changing appearance long
/// after the fleet itself repeats.
const PALETTES: [[u16; ROWS]; 6] = [
    [
        rgb(120, 230, 255),
        rgb(120, 180, 255),
        rgb(150, 140, 255),
        rgb(200, 130, 255),
    ],
    [
        rgb(255, 210, 90),
        rgb(255, 160, 70),
        rgb(250, 110, 80),
        rgb(220, 80, 120),
    ],
    [
        rgb(140, 250, 180),
        rgb(90, 230, 200),
        rgb(70, 200, 230),
        rgb(170, 240, 130),
    ],
    [
        rgb(255, 140, 210),
        rgb(230, 120, 255),
        rgb(170, 130, 255),
        rgb(255, 175, 235),
    ],
    [
        rgb(245, 235, 130),
        rgb(215, 245, 110),
        rgb(170, 225, 95),
        rgb(250, 205, 100),
    ],
    [
        rgb(210, 222, 236),
        rgb(160, 182, 205),
        rgb(120, 196, 216),
        rgb(235, 244, 252),
    ],
];

#[derive(Clone, Copy)]
struct Shot {
    x: i32,
    y: i32,
    live: bool,
}

const NO_SHOT: Shot = Shot {
    x: 0,
    y: 0,
    live: false,
};

pub struct Invaders {
    alive: [[bool; COLS]; ROWS],
    /// Bunker blocks, eroded from either side: bombs chew down through them and
    /// the gun shoots up through them, which is what makes hiding a decision
    /// rather than a free shelter.
    bunkers: [[[bool; BUNKER_W]; BUNKER_H]; BUNKERS],
    /// Fleet origin: the top-left invader's centre.
    fleet_x: i32,
    /// Q, and eased toward `fleet_drop_to`: the step down is a glide, not a jump.
    fleet_y: i32,
    fleet_drop_to: i32,
    fleet_dir: i32,
    ship_x: i32,
    shots: [Shot; MAX_SHOTS],
    bombs: [Shot; MAX_BOMBS],
    next_fire_ms: u32,
    next_bomb_ms: u32,
    pub wave: u32,
    lives: u8,
    score: u32,
    settle_ms: u32,
}

impl Invaders {
    pub const fn new() -> Self {
        Self {
            alive: [[true; COLS]; ROWS],
            bunkers: [[[true; BUNKER_W]; BUNKER_H]; BUNKERS],
            fleet_x: (W as i32 - (COLS as i32 - 1) * CELL_W) / 2,
            fleet_y: FLEET_TOP * Q,
            fleet_drop_to: FLEET_TOP * Q,
            fleet_dir: 1,
            ship_x: W as i32 / 2,
            shots: [NO_SHOT; MAX_SHOTS],
            bombs: [NO_SHOT; MAX_BOMBS],
            next_fire_ms: 0,
            next_bomb_ms: 0,
            wave: 0,
            lives: LIVES,
            score: 0,
            settle_ms: 0,
        }
    }

    fn palette(&self) -> &'static [u16; ROWS] {
        &PALETTES[self.wave as usize % PALETTES.len()]
    }

    pub fn restart(&mut self, now_ms: u32) {
        let ship_x = self.ship_x;
        *self = Self::new();
        self.ship_x = ship_x;
        self.settle_ms = now_ms + 600;
    }

    fn next_wave(&mut self, now_ms: u32) {
        let (wave, lives, score) = (self.wave + 1, self.lives, self.score);
        let ship_x = self.ship_x;
        *self = Self::new();
        self.wave = wave;
        self.lives = lives;
        self.score = score;
        self.ship_x = ship_x;
        self.settle_ms = now_ms + 600;
    }

    pub fn touch(&mut self, x: i32) {
        self.ship_x = x.clamp(SHIP_R, W as i32 - SHIP_R);
    }

    pub fn update(&mut self, dt_ms: u32, now_ms: u32, bubbles: &mut Bubbles) {
        if now_ms < self.settle_ms {
            return;
        }
        let dt = dt_ms.min(40) as i32;

        // Fleet. Speed rises with the wave and as the fleet thins, which is what
        // makes the last few invaders the dangerous ones.
        let left = self.alive.iter().flatten().filter(|a| **a).count().max(1);
        let thinning = 1 + (COLS * ROWS - left) as i32 / 6;
        let speed = (FLEET_SPEED + self.wave as i32 * 8 * Q) * thinning;
        self.fleet_x += self.fleet_dir * speed * dt / 1000 / Q;

        let (min_col, max_col) = self.occupied_columns();
        let leftmost = self.fleet_x + min_col * CELL_W;
        let rightmost = self.fleet_x + max_col * CELL_W;
        if leftmost < INVADER_R + 8 && self.fleet_dir < 0
            || rightmost > W as i32 - INVADER_R - 8 && self.fleet_dir > 0
        {
            self.fleet_dir = -self.fleet_dir;
            // Only the target moves here; the fleet slides down to it over the next
            // few frames, so reversing and dropping reads as one diagonal move
            // rather than a teleport.
            self.fleet_drop_to += FLEET_DROP * Q;
        }

        if self.fleet_y < self.fleet_drop_to {
            self.fleet_y = (self.fleet_y + DROP_SPEED * Q * dt / 1000).min(self.fleet_drop_to);
        }

        // Firing.
        if now_ms >= self.next_fire_ms {
            self.next_fire_ms = now_ms + FIRE_EVERY_MS;
            if let Some(slot) = self.shots.iter_mut().find(|s| !s.live) {
                *slot = Shot {
                    x: self.ship_x * Q,
                    y: (SHIP_Y - SHIP_R) * Q,
                    live: true,
                };
            }
        }
        if now_ms >= self.next_bomb_ms {
            self.next_bomb_ms = now_ms + BOMB_EVERY_MS;
            self.drop_bomb(now_ms);
        }

        for shot in self.shots.iter_mut().filter(|s| s.live) {
            shot.y -= SHOT_SPEED * dt / 1000;
            if shot.y < 0 {
                shot.live = false;
            }
        }
        for bomb in self.bombs.iter_mut().filter(|b| b.live) {
            bomb.y += BOMB_SPEED * dt / 1000;
            if bomb.y > (H as i32) * Q {
                bomb.live = false;
            }
        }

        self.erode(now_ms, bubbles);
        self.resolve_hits(now_ms, bubbles);

        // Bombs against the ship, and the fleet arriving.
        let ship_x = self.ship_x;
        let mut hit = false;
        for bomb in self.bombs.iter_mut().filter(|b| b.live) {
            if (bomb.x / Q - ship_x).abs() < SHIP_R + BOMB_R
                && (bomb.y / Q - SHIP_Y).abs() < SHIP_R
            {
                bomb.live = false;
                hit = true;
            }
        }
        let lowest = self.fleet_y / Q + self.lowest_row() * CELL_H;
        if hit || lowest > SHIP_Y - SHIP_R - INVADER_R {
            self.lives = self.lives.saturating_sub(1);
            bubbles.spawn(
                ship_x,
                SHIP_Y,
                now_ms,
                Some(muted(rgb(230, 70, 70))),
                Some(210),
                true,
            );
            if self.lives == 0 {
                self.restart(now_ms);
            } else {
                let (wave, lives, score) = (self.wave, self.lives, self.score);
                let alive = self.alive;
                let bunkers = self.bunkers;
                let (fx, fy, drop_to) = (self.fleet_x, self.fleet_y, self.fleet_drop_to);
                *self = Self::new();
                self.wave = wave;
                self.lives = lives;
                self.score = score;
                self.alive = alive;
                self.bunkers = bunkers;
                self.fleet_x = fx;
                self.fleet_y = fy;
                self.fleet_drop_to = drop_to;
                self.ship_x = ship_x;
                self.settle_ms = now_ms + 700;
            }
        }
    }

    fn drop_bomb(&mut self, now_ms: u32) {
        // From a random-ish living invader, chosen off the clock.
        let living: usize = self.alive.iter().flatten().filter(|a| **a).count();
        if living == 0 {
            return;
        }
        let pick = (now_ms as usize / 7) % living;
        let mut seen = 0;
        for row in 0..ROWS {
            for col in 0..COLS {
                if !self.alive[row][col] {
                    continue;
                }
                if seen == pick {
                    let (x, y) = self.invader_at(row, col);
                    if let Some(slot) = self.bombs.iter_mut().find(|b| !b.live) {
                        *slot = Shot {
                            x: x * Q,
                            y: y * Q,
                            live: true,
                        };
                    }
                    return;
                }
                seen += 1;
            }
        }
    }

    /// Bunker geometry, so drawing and collision share one source.
    fn block_rect(index: usize, col: usize, row: usize) -> (i32, i32, i32, i32) {
        let span = BUNKER_W as i32 * BLOCK;
        let gap = (W as i32 - BUNKERS as i32 * span) / (BUNKERS as i32 + 1);
        let x0 = gap + index as i32 * (span + gap) + col as i32 * BLOCK;
        let y0 = BUNKER_TOP + row as i32 * BLOCK;
        (x0, y0, x0 + BLOCK - 2, y0 + BLOCK - 2)
    }

    /// Anything in flight that is inside a block takes it away and stops there.
    fn erode(&mut self, now_ms: u32, bubbles: &mut Bubbles) {
        for index in 0..BUNKERS {
            for row in 0..BUNKER_H {
                for col in 0..BUNKER_W {
                    if !self.bunkers[index][row][col] {
                        continue;
                    }
                    let (x0, y0, x1, y1) = Self::block_rect(index, col, row);
                    let mut struck = false;
                    for shot in self.shots.iter_mut().filter(|s| s.live) {
                        let (x, y) = (shot.x / Q, shot.y / Q);
                        if x >= x0 && x <= x1 && y >= y0 && y <= y1 {
                            shot.live = false;
                            struck = true;
                        }
                    }
                    for bomb in self.bombs.iter_mut().filter(|b| b.live) {
                        let (x, y) = (bomb.x / Q, bomb.y / Q);
                        if x >= x0 && x <= x1 && y >= y0 && y <= y1 {
                            bomb.live = false;
                            struck = true;
                        }
                    }
                    if struck {
                        self.bunkers[index][row][col] = false;
                        bubbles.spawn(
                            (x0 + x1) / 2,
                            (y0 + y1) / 2,
                            now_ms,
                            Some(muted(rgb(140, 220, 160))),
                            Some(70),
                            true,
                        );
                    }
                }
            }
        }
    }

    fn resolve_hits(&mut self, now_ms: u32, bubbles: &mut Bubbles) {
        let palette = *self.palette();
        for index in 0..MAX_SHOTS {
            if !self.shots[index].live {
                continue;
            }
            let (sx, sy) = (self.shots[index].x / Q, self.shots[index].y / Q);
            let mut struck = None;
            for row in 0..ROWS {
                for col in 0..COLS {
                    if !self.alive[row][col] {
                        continue;
                    }
                    let (ix, iy) = self.invader_at(row, col);
                    if (sx - ix).abs() < INVADER_R + SHOT_R && (sy - iy).abs() < INVADER_R {
                        struck = Some((row, col, ix, iy));
                    }
                }
            }
            if let Some((row, col, ix, iy)) = struck {
                self.alive[row][col] = false;
                self.shots[index].live = false;
                self.score += 10 * (ROWS - row) as u32;
                bubbles.spawn(ix, iy, now_ms, Some(muted(palette[row])), Some(120), true);
            }
        }
        if !self.alive.iter().flatten().any(|a| *a) {
            bubbles.spawn(
                W as i32 / 2,
                H as i32 / 2,
                now_ms,
                Some(muted(palette[0])),
                None,
                true,
            );
            self.next_wave(now_ms);
        }
    }

    fn invader_at(&self, row: usize, col: usize) -> (i32, i32) {
        (
            self.fleet_x + col as i32 * CELL_W,
            self.fleet_y / Q + row as i32 * CELL_H,
        )
    }

    fn occupied_columns(&self) -> (i32, i32) {
        let mut min = COLS as i32 - 1;
        let mut max = 0;
        for row in &self.alive {
            for (col, alive) in row.iter().enumerate() {
                if *alive {
                    min = min.min(col as i32);
                    max = max.max(col as i32);
                }
            }
        }
        (min, max)
    }

    fn lowest_row(&self) -> i32 {
        let mut lowest = 0;
        for (row, line) in self.alive.iter().enumerate() {
            if line.iter().any(|a| *a) {
                lowest = row as i32;
            }
        }
        lowest
    }

    pub fn draw(&self, scene: &mut Scene, alpha: u8) {
        let palette = self.palette();
        for row in 0..ROWS {
            for col in 0..COLS {
                if !self.alive[row][col] {
                    continue;
                }
                let (x, y) = self.invader_at(row, col);
                // A body and two stubby arms, which is as much of an invader as
                // axis-aligned shapes allow - and reads as one.
                scene.pill(
                    x - INVADER_R,
                    y - INVADER_R / 2,
                    x + INVADER_R,
                    y + INVADER_R / 2,
                    5,
                    palette[row],
                    alpha,
                );
                scene.pill(
                    x - INVADER_R / 2,
                    y - INVADER_R,
                    x + INVADER_R / 2,
                    y + INVADER_R,
                    4,
                    palette[row],
                    alpha,
                );
            }
        }

        for index in 0..BUNKERS {
            for row in 0..BUNKER_H {
                for col in 0..BUNKER_W {
                    if !self.bunkers[index][row][col] {
                        continue;
                    }
                    let (x0, y0, x1, y1) = Self::block_rect(index, col, row);
                    scene.pill(x0, y0, x1, y1, 4, rgb(70, 150, 100), alpha);
                }
            }
        }

        for shot in self.shots.iter().filter(|s| s.live) {
            scene.disc(shot.x / Q, shot.y / Q, SHOT_R, rgb(255, 255, 240), alpha);
        }
        for bomb in self.bombs.iter().filter(|b| b.live) {
            scene.disc(bomb.x / Q, bomb.y / Q, BOMB_R, rgb(255, 120, 120), alpha);
        }

        scene.pill(
            self.ship_x - SHIP_R,
            SHIP_Y - 6,
            self.ship_x + SHIP_R,
            SHIP_Y + 8,
            6,
            rgb(150, 240, 190),
            alpha,
        );
        scene.pill(
            self.ship_x - 4,
            SHIP_Y - SHIP_R,
            self.ship_x + 4,
            SHIP_Y,
            3,
            rgb(220, 255, 235),
            alpha,
        );

        let mut line = TextBuf::new();
        use core::fmt::Write as _;
        let _ = write!(
            line,
            "WAVE {} \u{b7} {} \u{b7} {}",
            self.wave + 1,
            self.score,
            self.lives
        );
        scene.label(
            W as i32 / 2,
            76,
            FontId::Caption,
            palette[0],
            alpha,
            Align::Center,
            line.as_str(),
        );
    }
}
