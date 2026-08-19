//! Match three. Press a gem and drag it toward the neighbour you want it to
//! trade places with.
//!
//! A small state machine, because every step of a cascade has to be *seen*: the
//! swap slides, matches pop as rings, survivors fall into the gaps, and only then
//! does the board look for matches again. Resolving it all in one frame would be
//! correct and unreadable.
//!
//! Generating a board that is actually playable took three parts, and the first
//! version had only one of them:
//!
//! - Gems come from a small xorshift, not a rotation. A rotation was the original
//!   bug: cycling five kinds across a seven-wide board means no two neighbours are
//!   ever equal, and a board with no adjacent pair has no possible move at all.
//! - The initial fill rejects a kind that would *complete* a run, so the board
//!   does not open on a free cascade. Refills do not reject: a cascade you set up
//!   by clearing beneath is earned, and it is most of the fun.
//! - Then it checks that some single swap would match, and if none would, plants
//!   one. Planting three cells is gentler and more certain than reshuffling and
//!   hoping.

use crate::bubbles::Bubbles;
use crate::font::FontId;
use crate::gfx::{Align, Scene, TextBuf, W, muted, rgb};

const N: usize = 7;
const KINDS: usize = 5;

/// Sized by the space below the header and centred, so the last row is on screen.
const BOARD_TOP: i32 = 106;
const CELL: i32 = 50;
const MARGIN: i32 = (W as i32 - N as i32 * CELL) / 2;
const GEM_R: i32 = CELL / 2 - 5;

const SWAP_MS: u32 = 130;
const POP_MS: u32 = 170;
const FALL_MS: u32 = 170;
/// How far the finger must travel before a drag counts as a direction.
const DRAG_MIN: i32 = 18;

const COLORS: [u16; KINDS] = [
    rgb(255, 96, 120),
    rgb(255, 190, 70),
    rgb(110, 220, 150),
    rgb(90, 180, 255),
    rgb(200, 130, 255),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Idle,
    /// Swapping the selected pair; `revert` when the swap made no match.
    Swap {
        a: (usize, usize),
        b: (usize, usize),
        revert: bool,
    },
    Pop,
    Fall,
}

pub struct Match3 {
    /// 0 is an empty cell, otherwise the gem kind plus one.
    cells: [[u8; N]; N],
    /// Where each gem is falling from, in cells; 0 once it has landed.
    drop_from: [[i8; N]; N],
    doomed: [[bool; N]; N],
    step: Step,
    step_started_ms: u32,
    /// The gem under the finger, and where the finger went down. A gesture is
    /// consumed as soon as it commits to a direction, so one drag is one swap.
    selected: Option<(usize, usize)>,
    drag_origin: (i32, i32),
    score: u32,
    chain: u32,
    rng: u32,
}

impl Match3 {
    pub const fn new() -> Self {
        Self {
            cells: [[0; N]; N],
            drop_from: [[0; N]; N],
            doomed: [[false; N]; N],
            step: Step::Idle,
            step_started_ms: 0,
            selected: None,
            drag_origin: (0, 0),
            score: 0,
            chain: 0,
            rng: 0x2545_f491,
        }
    }

    pub fn restart(&mut self, now_ms: u32) {
        *self = Self::new();
        self.rng = now_ms | 1;
        self.deal();
    }

    fn next_rand(&mut self) -> u32 {
        // xorshift32: three shifts, no state beyond the word itself.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    fn random_kind(&mut self) -> u8 {
        (self.next_rand() % KINDS as u32) as u8 + 1
    }

    /// Fill, then make sure there is something to do.
    fn deal(&mut self) {
        for r in 0..N {
            for c in 0..N {
                self.cells[r][c] = self.fill_for(r, c);
            }
        }
        if !self.has_move() {
            self.plant_move();
        }
    }

    /// Put a guaranteed move on the board: a pair, and a third gem one square off
    /// the end of it, so sliding that third into line completes a row.
    ///
    /// Three cells change, wherever it lands - far less disruptive than dealing the
    /// whole board again, and unlike a reshuffle it cannot fail to help.
    fn plant_move(&mut self) -> bool {
        for _ in 0..48 {
            let r = (self.next_rand() as usize) % (N - 1);
            let c = (self.next_rand() as usize) % (N - 2);
            let kind = self.random_kind();
            let keep = [
                self.cells[r][c],
                self.cells[r][c + 1],
                self.cells[r][c + 2],
                self.cells[r + 1][c + 2],
            ];
            self.cells[r][c] = kind;
            self.cells[r][c + 1] = kind;
            self.cells[r + 1][c + 2] = kind;
            // The gap must not already hold this kind, or the row is a match on
            // sight instead of a move.
            if self.cells[r][c + 2] == kind {
                self.cells[r][c + 2] = kind % KINDS as u8 + 1;
            }
            if !self.any_match() && self.has_move() {
                return true;
            }
            self.cells[r][c] = keep[0];
            self.cells[r][c + 1] = keep[1];
            self.cells[r][c + 2] = keep[2];
            self.cells[r + 1][c + 2] = keep[3];
        }
        false
    }

    /// Whether any single swap of neighbours would make a match. Tries each of
    /// them and puts the board back, which is exact - the alternative is a
    /// pattern-matching approximation that eventually disagrees with the rules the
    /// clearing code actually uses.
    fn has_move(&mut self) -> bool {
        for r in 0..N {
            for c in 0..N {
                for (dr, dc) in [(0usize, 1usize), (1, 0)] {
                    let (r2, c2) = (r + dr, c + dc);
                    if r2 >= N || c2 >= N {
                        continue;
                    }
                    let keep = self.cells[r][c];
                    self.cells[r][c] = self.cells[r2][c2];
                    self.cells[r2][c2] = keep;
                    let found = self.any_match();
                    let keep = self.cells[r][c];
                    self.cells[r][c] = self.cells[r2][c2];
                    self.cells[r2][c2] = keep;
                    if found {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Read-only twin of `mark_matches`, for asking without changing anything.
    fn any_match(&self) -> bool {
        for r in 0..N {
            for c in 2..N {
                let kind = self.cells[r][c];
                if kind != 0 && self.cells[r][c - 1] == kind && self.cells[r][c - 2] == kind {
                    return true;
                }
            }
        }
        for c in 0..N {
            for r in 2..N {
                let kind = self.cells[r][c];
                if kind != 0 && self.cells[r - 1][c] == kind && self.cells[r - 2][c] == kind {
                    return true;
                }
            }
        }
        false
    }

    /// A random kind that does not complete a run at (r, c) - for the opening
    /// board, which should not begin by clearing itself.
    fn fill_for(&mut self, r: usize, c: usize) -> u8 {
        for _ in 0..8 {
            let kind = self.random_kind();
            let two_left = c >= 2 && self.cells[r][c - 1] == kind && self.cells[r][c - 2] == kind;
            let two_up = r >= 2 && self.cells[r - 1][c] == kind && self.cells[r - 2][c] == kind;
            if !two_left && !two_up {
                return kind;
            }
        }
        self.random_kind()
    }

    pub fn press(&mut self, x: i32, y: i32) {
        if self.step != Step::Idle {
            return;
        }
        let col = (x - MARGIN) / CELL;
        let row = (y - BOARD_TOP) / CELL;
        if col < 0 || row < 0 || col >= N as i32 || row >= N as i32 {
            return;
        }
        self.selected = Some((row as usize, col as usize));
        self.drag_origin = (x, y);
    }

    /// Commit the moment the drag has a direction, rather than waiting for the
    /// finger to lift: a swap should happen while you are still pushing.
    pub fn drag(&mut self, x: i32, y: i32, now_ms: u32) {
        let Some(from) = self.selected else {
            return;
        };
        if self.step != Step::Idle {
            return;
        }
        let (dx, dy) = (x - self.drag_origin.0, y - self.drag_origin.1);
        if dx.abs().max(dy.abs()) < DRAG_MIN {
            return;
        }
        let (dr, dc) = if dx.abs() > dy.abs() {
            (0, dx.signum())
        } else {
            (dy.signum(), 0)
        };
        let (r, c) = (from.0 as i32 + dr, from.1 as i32 + dc);
        if r < 0 || c < 0 || r >= N as i32 || c >= N as i32 {
            self.selected = None;
            return;
        }
        self.begin_swap(from, (r as usize, c as usize), now_ms);
    }

    pub fn release(&mut self) {
        self.selected = None;
    }

    fn begin_swap(&mut self, a: (usize, usize), b: (usize, usize), now_ms: u32) {
        self.selected = None;
        let temp = self.cells[a.0][a.1];
        self.cells[a.0][a.1] = self.cells[b.0][b.1];
        self.cells[b.0][b.1] = temp;
        // Whether it stays is decided now; the slide is just how it is presented.
        let revert = !self.mark_matches();
        self.doomed = [[false; N]; N];
        self.chain = 0;
        self.step = Step::Swap { a, b, revert };
        self.step_started_ms = now_ms;
    }

    /// Flags every gem in a run of three or more. True if anything matched.
    fn mark_matches(&mut self) -> bool {
        let mut any = false;
        for r in 0..N {
            let mut run = 1;
            for c in 1..=N {
                let same = c < N && self.cells[r][c] != 0 && self.cells[r][c] == self.cells[r][c - 1];
                if same {
                    run += 1;
                    continue;
                }
                if run >= 3 {
                    for k in 0..run {
                        self.doomed[r][c - 1 - k] = true;
                    }
                    any = true;
                }
                run = 1;
            }
        }
        for c in 0..N {
            let mut run = 1;
            for r in 1..=N {
                let same = r < N && self.cells[r][c] != 0 && self.cells[r][c] == self.cells[r - 1][c];
                if same {
                    run += 1;
                    continue;
                }
                if run >= 3 {
                    for k in 0..run {
                        self.doomed[r - 1 - k][c] = true;
                    }
                    any = true;
                }
                run = 1;
            }
        }
        any
    }

    pub fn update(&mut self, now_ms: u32, bubbles: &mut Bubbles) {
        let age = now_ms.wrapping_sub(self.step_started_ms);
        match self.step {
            Step::Idle => {}
            Step::Swap { a, b, revert } => {
                if age < SWAP_MS {
                    return;
                }
                if revert {
                    let temp = self.cells[a.0][a.1];
                    self.cells[a.0][a.1] = self.cells[b.0][b.1];
                    self.cells[b.0][b.1] = temp;
                    self.step = Step::Idle;
                } else {
                    self.mark_matches();
                    self.burst(now_ms, bubbles);
                    self.step = Step::Pop;
                    self.step_started_ms = now_ms;
                }
            }
            Step::Pop => {
                if age < POP_MS {
                    return;
                }
                self.collapse();
                self.step = Step::Fall;
                self.step_started_ms = now_ms;
            }
            Step::Fall => {
                if age < FALL_MS {
                    return;
                }
                self.drop_from = [[0; N]; N];
                // Cascades: whatever the fall lined up counts too, and scores more.
                if self.mark_matches() {
                    self.chain += 1;
                    self.burst(now_ms, bubbles);
                    self.step = Step::Pop;
                    self.step_started_ms = now_ms;
                } else {
                    self.doomed = [[false; N]; N];
                    // A refill can settle into a board with nothing to do; deal
                    // again rather than leave it stuck.
                    if !self.has_move() {
                        self.plant_move();
                    }
                    self.step = Step::Idle;
                }
            }
        }
    }

    /// One ring per cleared gem, in its own colour - the juice.
    fn burst(&mut self, now_ms: u32, bubbles: &mut Bubbles) {
        let mut cleared = 0;
        for r in 0..N {
            for c in 0..N {
                if !self.doomed[r][c] {
                    continue;
                }
                let kind = self.cells[r][c];
                if kind == 0 {
                    continue;
                }
                let (x, y) = centre(r, c);
                bubbles.spawn(
                    x,
                    y,
                    now_ms,
                    Some(muted(COLORS[(kind - 1) as usize % KINDS])),
                    Some(CELL + 12),
                    true,
                );
                cleared += 1;
            }
        }
        // Longer runs and deeper chains are worth more, which is what makes
        // setting one up worthwhile.
        self.score += cleared * 10 * (self.chain + 1);
    }

    /// Doomed gems go, survivors fall, the top refills. `drop_from` records how
    /// far each gem has to travel so the fall can be drawn.
    fn collapse(&mut self) {
        for c in 0..N {
            let mut write = N as i32 - 1;
            for r in (0..N).rev() {
                if self.doomed[r][c] {
                    continue;
                }
                let value = self.cells[r][c];
                if value == 0 {
                    continue;
                }
                self.cells[write as usize][c] = value;
                self.drop_from[write as usize][c] = (write - r as i32) as i8;
                write -= 1;
            }
            // Everything above the write head is new, and arrives from off-screen.
            let mut above = write;
            while above >= 0 {
                self.cells[above as usize][c] = 0;
                above -= 1;
            }
            let mut above = write;
            while above >= 0 {
                let kind = self.random_kind();
                self.cells[above as usize][c] = kind;
                self.drop_from[above as usize][c] = (above + 2) as i8;
                above -= 1;
            }
        }
        self.doomed = [[false; N]; N];
    }

    pub fn draw(&self, scene: &mut Scene, now_ms: u32, alpha: u8) {
        scene.pill(
            MARGIN - 6,
            BOARD_TOP - 6,
            W as i32 - MARGIN + 6,
            BOARD_TOP + N as i32 * CELL + 6,
            12,
            rgb(16, 18, 24),
            alpha,
        );

        let age = now_ms.wrapping_sub(self.step_started_ms);
        for r in 0..N {
            for c in 0..N {
                let kind = self.cells[r][c];
                if kind == 0 {
                    continue;
                }
                let (mut x, mut y) = centre(r, c);
                let mut radius = GEM_R;

                match self.step {
                    // Slide the two swapping gems along their path.
                    Step::Swap { a, b, .. } => {
                        let t = (age * 256 / SWAP_MS).min(256) as i32;
                        if (r, c) == a || (r, c) == b {
                            let other = if (r, c) == a { b } else { a };
                            let (ox, oy) = centre(other.0, other.1);
                            x += (ox - x) * (256 - t) / 256;
                            y += (oy - y) * (256 - t) / 256;
                        }
                    }
                    // Doomed gems shrink away.
                    Step::Pop if self.doomed[r][c] => {
                        let t = (age * 256 / POP_MS).min(256) as i32;
                        radius = radius * (256 - t) / 256;
                    }
                    Step::Fall => {
                        let travel = self.drop_from[r][c] as i32;
                        if travel > 0 {
                            let t = (age * 256 / FALL_MS).min(256) as i32;
                            y -= travel * CELL * (256 - t) / 256;
                        }
                    }
                    _ => {}
                }
                if radius <= 0 {
                    continue;
                }
                scene.disc(x, y, radius, COLORS[(kind - 1) as usize % KINDS], alpha);
                // A highlight, so a flat disc reads as a gem.
                scene.disc(
                    x - radius / 3,
                    y - radius / 3,
                    radius / 4,
                    rgb(255, 255, 255),
                    alpha / 2,
                );
            }
        }

        if let Some((r, c)) = self.selected {
            let (x, y) = centre(r, c);
            scene.ring(x, y, GEM_R + 5, GEM_R + 2, rgb(255, 255, 255), alpha);
        }

        let mut line = TextBuf::new();
        use core::fmt::Write as _;
        let _ = write!(line, "{}", self.score);
        scene.label(
            W as i32 / 2,
            76,
            FontId::Body,
            rgb(230, 234, 240),
            alpha,
            Align::Center,
            line.as_str(),
        );
    }
}

fn centre(row: usize, col: usize) -> (i32, i32) {
    (
        MARGIN + col as i32 * CELL + CELL / 2,
        BOARD_TOP + row as i32 * CELL + CELL / 2,
    )
}
