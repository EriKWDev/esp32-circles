//! Pac-Man.
//!
//! The maze is ASCII, one string per row, because that is the form it can be read
//! and corrected in - the same reason the chess book is text.
//!
//! It is also deliberately smaller than the arcade's. That maze has some 240
//! pellets and the scene holds 176 primitives in total: a dot each would draw
//! about half a maze and no ghosts. This one is sized so that its pellets, its
//! walls merged into horizontal runs, the ghosts and the chrome all fit inside the
//! budget at once - which the host check counts rather than trusts.
//!
//! Steering is by wedge, as the worm's is: press the top of the screen to go up.
//! The maze covers the panel, so there is nowhere to put a d-pad that would not
//! cover the maze.

use crate::font::FontId;
use crate::gfx::{Align, Scene, TextBuf, W, muted, rgb};

pub const COLS: usize = 15;
pub const ROWS: usize = 14;
const CELL: i32 = 26;
const BOARD_X: i32 = (W as i32 - CELL * COLS as i32) / 2;
/// Below the Back button, and the whole maze above the bottom edge.
const BOARD_Y: i32 = 100;

/// `#` wall, `.` pellet, `o` power pellet, `P` Pac-Man's start, `G` a ghost's.
pub const MAZE: [&str; ROWS] = [
    "###############",
    "#......#......#",
    "#o##.#.#.#.##o#",
    "#..#.#...#.#..#",
    "##.#.##.##.#.##",
    "#.....GGG.....#",
    "##.#.#####.#.##",
    "#..#...P...#..#",
    "#.####.#.####.#",
    "#......#......#",
    "#.##.#####.##.#",
    "#o.#...#...#.o#",
    "##.#.#####.#.##",
    "###############",
];

const MAX_GHOSTS: usize = 3;
/// Pixels per second, at Q. Pac-Man is a little quicker than the ghosts, which is
/// what makes a corner worth taking.
const Q: i32 = 16;
const PAC_SPEED: i32 = 62 * Q;
const GHOST_SPEED: i32 = 52 * Q;
const FRIGHT_SPEED: i32 = 34 * Q;
const FRIGHT_MS: u32 = 6_500;
const LIVES: u8 = 3;
/// How long the board is held still after a death, and after clearing it.
const PAUSE_MS: u32 = 1_100;

const WALL: u16 = rgb(48, 72, 200);
const PELLET: u16 = rgb(240, 226, 180);
const PAC: u16 = rgb(250, 220, 60);
const INK: u16 = rgb(238, 245, 250);
const DIM: u16 = rgb(140, 152, 166);
const FRIGHT: u16 = rgb(90, 120, 240);
pub const ACCENT: u16 = rgb(250, 220, 60);
const GHOST_COLORS: [u16; MAX_GHOSTS] = [rgb(240, 80, 70), rgb(250, 160, 220), rgb(90, 220, 230)];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Ready,
    Playing,
    /// Caught, or the board is clear; the pause reads as a beat either way.
    Paused,
    Over,
}

#[derive(Clone, Copy)]
struct Mover {
    /// Pixel position within the board, at Q.
    x: i32,
    y: i32,
    dx: i8,
    dy: i8,
    /// Chosen but not yet possible; taken at the next cell centre.
    want: (i8, i8),
}

impl Mover {
    const fn at(col: i32, row: i32) -> Self {
        Self {
            x: (col * CELL + CELL / 2) * Q,
            y: (row * CELL + CELL / 2) * Q,
            dx: 0,
            dy: 0,
            want: (0, 0),
        }
    }

    fn cell(&self) -> (i32, i32) {
        (self.x / Q / CELL, self.y / Q / CELL)
    }

    /// How far past the middle of its cell, so a turn only happens where the
    /// corridors actually meet.
    fn centred(&self) -> bool {
        let (col, row) = self.cell();
        let cx = (col * CELL + CELL / 2) * Q;
        let cy = (row * CELL + CELL / 2) * Q;
        (self.x - cx).abs() < PAC_SPEED / 40 + Q && (self.y - cy).abs() < PAC_SPEED / 40 + Q
    }

    fn snap(&mut self) {
        let (col, row) = self.cell();
        self.x = (col * CELL + CELL / 2) * Q;
        self.y = (row * CELL + CELL / 2) * Q;
    }
}

pub struct Pacman {
    /// Remaining pellets: 1 plain, 2 power, 0 eaten or never there.
    dots: [[u8; COLS]; ROWS],
    left: u32,
    pac: Mover,
    ghosts: [Mover; MAX_GHOSTS],
    phase: Phase,
    fright_until_ms: u32,
    resume_at_ms: u32,
    lives: u8,
    score: u32,
    pub best: u32,
    /// Bumped per level, which is all "faster" means here.
    level: u32,
    seed: u32,
}

impl Pacman {
    pub const fn new() -> Self {
        Self {
            dots: [[0; COLS]; ROWS],
            left: 0,
            pac: Mover::at(7, 7),
            ghosts: [Mover::at(6, 5), Mover::at(7, 5), Mover::at(8, 5)],
            phase: Phase::Ready,
            fright_until_ms: 0,
            resume_at_ms: 0,
            lives: LIVES,
            score: 0,
            best: 0,
            level: 0,
            seed: 0x51ed_2c9b,
        }
    }

    pub fn restart(&mut self, now_ms: u32) {
        let best = self.best;
        *self = Self::new();
        self.best = best;
        self.seed ^= now_ms | 1;
        self.fill();
        self.phase = Phase::Ready;
    }

    /// Pellets from the maze, and everyone back to their corner.
    fn fill(&mut self) {
        self.left = 0;
        for (row, line) in MAZE.iter().enumerate() {
            for (col, ch) in line.bytes().enumerate() {
                self.dots[row][col] = match ch {
                    b'.' => 1,
                    b'o' => 2,
                    _ => 0,
                };
                if self.dots[row][col] != 0 {
                    self.left += 1;
                }
            }
        }
        self.place();
    }

    fn place(&mut self) {
        for (row, line) in MAZE.iter().enumerate() {
            for (col, ch) in line.bytes().enumerate() {
                if ch == b'P' {
                    self.pac = Mover::at(col as i32, row as i32);
                }
            }
        }
        let mut index = 0;
        for (row, line) in MAZE.iter().enumerate() {
            for (col, ch) in line.bytes().enumerate() {
                if ch == b'G' && index < MAX_GHOSTS {
                    self.ghosts[index] = Mover::at(col as i32, row as i32);
                    index += 1;
                }
            }
        }
    }

    pub fn tap(&mut self, now_ms: u32) {
        match self.phase {
            Phase::Ready => self.phase = Phase::Playing,
            Phase::Over => self.restart(now_ms),
            _ => {}
        }
    }

    /// Steer by wedge from the middle of the maze: the larger offset wins, so the
    /// four regions are triangles covering the whole panel.
    pub fn steer(&mut self, x: i32, y: i32) {
        let dx = x - (BOARD_X + CELL * COLS as i32 / 2);
        let dy = y - (BOARD_Y + CELL * ROWS as i32 / 2);
        self.pac.want = if dx.abs() > dy.abs() {
            (dx.signum() as i8, 0)
        } else {
            (0, dy.signum() as i8)
        };
    }

    fn wall(col: i32, row: i32) -> bool {
        if row < 0 || col < 0 || row >= ROWS as i32 || col >= COLS as i32 {
            return true;
        }
        MAZE[row as usize].as_bytes()[col as usize] == b'#'
    }

    /// Whether a mover may leave its cell in this direction.
    fn open(mover: &Mover, dx: i8, dy: i8) -> bool {
        if dx == 0 && dy == 0 {
            return false;
        }
        let (col, row) = mover.cell();
        !Self::wall(col + dx as i32, row + dy as i32)
    }

    fn advance(mover: &mut Mover, speed: i32, dt: i32) {
        // Turns happen at cell centres only, which is what keeps a mover in its
        // corridor rather than cutting a corner into a wall.
        if mover.centred() {
            let (wx, wy) = mover.want;
            if (wx, wy) != (0, 0) && Self::open(mover, wx, wy) {
                mover.snap();
                mover.dx = wx;
                mover.dy = wy;
            } else if !Self::open(mover, mover.dx, mover.dy) {
                mover.snap();
                mover.dx = 0;
                mover.dy = 0;
            }
        }
        mover.x += mover.dx as i32 * speed * dt / 1000;
        mover.y += mover.dy as i32 * speed * dt / 1000;
    }

    /// Where a ghost goes at a junction: towards Pac-Man, or away while
    /// frightened, and never straight back the way it came.
    fn steer_ghost(&mut self, index: usize, frightened: bool) {
        let ghost = self.ghosts[index];
        if !ghost.centred() {
            return;
        }
        let (gc, gr) = ghost.cell();
        let (pc, pr) = self.pac.cell();
        let mut best = (0i8, 0i8);
        let mut best_score = i32::MIN;
        for (dx, dy) in [(0i8, -1i8), (1, 0), (0, 1), (-1, 0)] {
            // Reversing is only allowed when there is nowhere else, which is what
            // stops a ghost jittering in a corridor.
            if (dx, dy) == (-ghost.dx, -ghost.dy) {
                continue;
            }
            if Self::wall(gc + dx as i32, gr + dy as i32) {
                continue;
            }
            let distance =
                (gc + dx as i32 - pc).abs() * 10 + (gr + dy as i32 - pr).abs() * 10;
            // A little noise, so three ghosts do not walk in one line.
            self.seed ^= self.seed << 13;
            self.seed ^= self.seed >> 17;
            self.seed ^= self.seed << 5;
            let jitter = (self.seed % 7) as i32;
            let score = if frightened {
                distance + jitter
            } else {
                -distance + jitter
            };
            if score > best_score {
                best_score = score;
                best = (dx, dy);
            }
        }
        if best != (0, 0) {
            self.ghosts[index].want = best;
        }
    }

    pub fn update(&mut self, dt_ms: u32, now_ms: u32, bubbles: &mut crate::bubbles::Bubbles) {
        if self.phase == Phase::Paused {
            if now_ms.wrapping_sub(self.resume_at_ms) < u32::MAX / 2 {
                self.phase = Phase::Playing;
            }
            return;
        }
        if self.phase != Phase::Playing {
            return;
        }
        let dt = dt_ms.min(40) as i32;
        let frightened = now_ms.wrapping_sub(self.fright_until_ms) > u32::MAX / 2;

        let mut pac = self.pac;
        Self::advance(&mut pac, PAC_SPEED + self.level as i32 * 2 * Q, dt);
        self.pac = pac;

        // Eating.
        let (col, row) = self.pac.cell();
        if row >= 0 && col >= 0 && (row as usize) < ROWS && (col as usize) < COLS {
            let dot = self.dots[row as usize][col as usize];
            if dot != 0 {
                self.dots[row as usize][col as usize] = 0;
                self.left = self.left.saturating_sub(1);
                self.score += if dot == 2 { 50 } else { 10 };
                self.best = self.best.max(self.score);
                if dot == 2 {
                    self.fright_until_ms = now_ms + FRIGHT_MS;
                    bubbles.spawn(
                        BOARD_X + col * CELL + CELL / 2,
                        BOARD_Y + row * CELL + CELL / 2,
                        now_ms,
                        Some(muted(PAC)),
                        Some(120),
                        true,
                    );
                }
            }
        }

        for index in 0..MAX_GHOSTS {
            self.steer_ghost(index, frightened);
            let mut ghost = self.ghosts[index];
            let speed = if frightened { FRIGHT_SPEED } else { GHOST_SPEED };
            Self::advance(&mut ghost, speed, dt);
            self.ghosts[index] = ghost;

            // Touching, in pixels rather than cells: a cell test would miss two
            // movers passing through each other between frames.
            let near = (self.pac.x - ghost.x).abs() < CELL * Q * 2 / 3
                && (self.pac.y - ghost.y).abs() < CELL * Q * 2 / 3;
            if !near {
                continue;
            }
            if frightened {
                self.score += 200;
                self.best = self.best.max(self.score);
                bubbles.spawn(
                    BOARD_X + ghost.x / Q,
                    BOARD_Y + ghost.y / Q,
                    now_ms,
                    Some(GHOST_COLORS[index]),
                    Some(140),
                    true,
                );
                self.place_one(index);
            } else {
                self.lives = self.lives.saturating_sub(1);
                bubbles.spawn(
                    BOARD_X + self.pac.x / Q,
                    BOARD_Y + self.pac.y / Q,
                    now_ms,
                    Some(PAC),
                    None,
                    true,
                );
                if self.lives == 0 {
                    self.phase = Phase::Over;
                } else {
                    self.place();
                    self.phase = Phase::Paused;
                    self.resume_at_ms = now_ms + PAUSE_MS;
                }
                return;
            }
        }

        if self.left == 0 {
            self.level += 1;
            self.fill();
            self.phase = Phase::Paused;
            self.resume_at_ms = now_ms + PAUSE_MS;
        }
    }

    /// One ghost back to its corner, after being eaten.
    fn place_one(&mut self, index: usize) {
        let mut slot = 0;
        for (row, line) in MAZE.iter().enumerate() {
            for (col, ch) in line.bytes().enumerate() {
                if ch == b'G' {
                    if slot == index {
                        self.ghosts[index] = Mover::at(col as i32, row as i32);
                        return;
                    }
                    slot += 1;
                }
            }
        }
    }

    pub fn draw(&self, scene: &mut Scene, now_ms: u32, alpha: u8) {
        // Walls as merged horizontal runs: a maze drawn cell by cell would spend
        // ninety primitives on what thirty describe.
        for (row, line) in MAZE.iter().enumerate() {
            let bytes = line.as_bytes();
            let mut col = 0;
            while col < COLS {
                if bytes[col] != b'#' {
                    col += 1;
                    continue;
                }
                let mut end = col;
                while end + 1 < COLS && bytes[end + 1] == b'#' {
                    end += 1;
                }
                let x0 = BOARD_X + col as i32 * CELL + 3;
                let y0 = BOARD_Y + row as i32 * CELL + 3;
                scene.pill(
                    x0,
                    y0,
                    BOARD_X + (end + 1) as i32 * CELL - 3,
                    y0 + CELL - 6,
                    5,
                    WALL,
                    alpha,
                );
                col = end + 1;
            }
        }

        for row in 0..ROWS {
            for col in 0..COLS {
                let dot = self.dots[row][col];
                if dot == 0 {
                    continue;
                }
                // A power pellet blinks, which is the only way to tell the two
                // apart at this size.
                let big = dot == 2;
                if big && now_ms / 250 % 2 == 0 {
                    continue;
                }
                scene.disc(
                    BOARD_X + col as i32 * CELL + CELL / 2,
                    BOARD_Y + row as i32 * CELL + CELL / 2,
                    if big { 6 } else { 2 },
                    PELLET,
                    alpha,
                );
            }
        }

        let frightened = now_ms.wrapping_sub(self.fright_until_ms) > u32::MAX / 2;
        for (index, ghost) in self.ghosts.iter().enumerate() {
            let color = if frightened {
                FRIGHT
            } else {
                GHOST_COLORS[index]
            };
            let (x, y) = (BOARD_X + ghost.x / Q, BOARD_Y + ghost.y / Q);
            scene.disc(x, y, CELL / 2 - 4, color, alpha);
            // Two eyes, so which way it is looking is visible.
            scene.disc(x - 3 + ghost.dx as i32 * 3, y - 3, 2, rgb(12, 14, 20), alpha);
            scene.disc(x + 4 + ghost.dx as i32 * 3, y - 3, 2, rgb(12, 14, 20), alpha);
        }

        // Pac-Man: a disc with a wedge bitten out of it by the background, which
        // costs one more primitive and no new shape.
        let (px, py) = (BOARD_X + self.pac.x / Q, BOARD_Y + self.pac.y / Q);
        scene.disc(px, py, CELL / 2 - 3, PAC, alpha);
        if now_ms / 110 % 2 == 0 {
            scene.disc(
                px + self.pac.dx as i32 * 7,
                py + self.pac.dy as i32 * 7,
                5,
                rgb(0, 0, 0),
                alpha,
            );
        }

        let mut line = TextBuf::new();
        use core::fmt::Write as _;
        let _ = write!(line, "{}  BEST {}", self.score, self.best);
        scene.label(
            W as i32 / 2 + 30,
            76,
            FontId::Caption,
            INK,
            alpha,
            Align::Center,
            line.as_str(),
        );
        for life in 0..self.lives as i32 {
            scene.disc(W as i32 - 26 - life * 22, 480 - 14, 7, PAC, alpha);
        }

        if self.phase == Phase::Ready || self.phase == Phase::Over {
            let over = self.phase == Phase::Over;
            scene.pill(96, 250, 384, 306, 28, rgb(6, 8, 12), alpha);
            scene.label(
                W as i32 / 2,
                286,
                FontId::Body,
                if over { GHOST_COLORS[0] } else { DIM },
                alpha,
                Align::Center,
                if over { "TAP TO RETRY" } else { "TAP TO START" },
            );
        }
    }
}
