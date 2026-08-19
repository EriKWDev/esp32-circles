//! Chess against the panel.
//!
//! The board is a 10x12 mailbox rather than a bare 8x8: the two-square border
//! makes every off-board test a single array lookup, which is what keeps knight
//! and sliding generation free of edge arithmetic and the bugs that come with it.
//!
//! The search is negamax with alpha-beta and iterative deepening, and it runs in
//! slices. It has to: the think time goes up to twenty seconds and this is the
//! same thread that reads the touch panel and services Wi-Fi, so `think` returns
//! after a fixed number of nodes and is called again next frame. The screen keeps
//! animating and the controller keeps being polled while the machine considers.
//!
//! Castling, en passant and under-promotion are all here - the first two because
//! a chess program without them is not one, the third because leaving it out was
//! measurable: perft came up 11,379 nodes short of the published count for the
//! standard test position, and a rook or knight is occasionally the only
//! promotion that does not stalemate. A human tapping a square still gets a
//! queen; the search considers all four. Draw by repetition and the fifty-move
//! rule are not here, which for a panel in a garden shed seems a fair trade.

use core::fmt::Write as _;

use crate::font::FontId;
use crate::gfx::{Align, H, Scene, TextBuf, W, muted, rgb};

/// Mailbox squares. Rank 8 begins at 21; A1 is 91.
const BOARD: usize = 120;
const START: usize = 21;

const EMPTY: i8 = 0;
const PAWN: i8 = 1;
const KNIGHT: i8 = 2;
const BISHOP: i8 = 3;
const ROOK: i8 = 4;
const QUEEN: i8 = 5;
const KING: i8 = 6;
/// Anything outside the playable 8x8, so a step off the edge is a single test.
const EDGE: i8 = 99;

const MAX_MOVES: usize = 218;
const MAX_PLY: usize = 24;
/// Nodes per call. The host manages thirteen million a second; this chip is two
/// orders of magnitude slower, so a few hundred nodes is about one frame's worth
/// of work - and the log reports the real rate per depth so it can be retuned.
const NODES_PER_SLICE: u32 = 1_200;

const VALUE: [i32; 7] = [0, 100, 320, 330, 500, 900, 20_000];
/// A pawn's worth of preference for the middle, by rank and file distance from
/// the centre - enough that the machine develops rather than shuffling.
const CENTRE: [i32; 8] = [-20, -5, 5, 12, 12, 5, -5, -20];

const OFFSETS: [[i8; 8]; 7] = [
    [0; 8],
    [0; 8],
    [-21, -19, -12, -8, 8, 12, 19, 21],
    [-11, -9, 9, 11, 0, 0, 0, 0],
    [-10, -1, 1, 10, 0, 0, 0, 0],
    [-11, -10, -9, -1, 1, 9, 10, 11],
    [-11, -10, -9, -1, 1, 9, 10, 11],
];
const N_OFFSETS: [usize; 7] = [0, 0, 8, 4, 4, 8, 8];
const SLIDES: [bool; 7] = [false, false, false, true, true, true, false];

pub const THINK_CHOICES: [u16; 4] = [1, 5, 10, 20];

const LIGHT: u16 = rgb(232, 220, 196);
const DARK: u16 = rgb(126, 96, 70);
const WHITE_PIECE: u16 = rgb(248, 248, 244);
const BLACK_PIECE: u16 = rgb(28, 28, 32);
const HINT: u16 = rgb(120, 220, 150);
const PICK: u16 = rgb(255, 206, 84);
const INK: u16 = rgb(238, 245, 250);
pub const ACCENT: u16 = rgb(214, 186, 140);

/// Board geometry. Sized so the board ends above the bottom edge, with the status
/// line *above* it rather than below: there is no room under an eight-square board
/// on a 480-pixel panel, and a centred line at this height clears the Back button
/// in the corner.
const CELL: i32 = 48;
const BOARD_X: i32 = (W as i32 - CELL * 8) / 2;
const BOARD_Y: i32 = 88;
const STATUS_Y: i32 = 66;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Choosing sides and think time.
    Setup,
    HumanTurn,
    Thinking,
    /// Game over; the text says how.
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Move {
    from: u8,
    to: u8,
    /// Piece captured, for unmaking. En passant records the pawn it took.
    taken: i8,
    /// The piece a pawn became, or EMPTY. Generated queen-first, so the human's
    /// tap on a destination square finds the queen.
    promote: i8,
    /// Set when this capture removes a pawn that is not on `to`.
    en_passant: bool,
    /// Set for a king moving two files; the rook is moved with it.
    castle: bool,
}

const NO_MOVE: Move = Move {
    from: 0,
    to: 0,
    taken: EMPTY,
    promote: EMPTY,
    en_passant: false,
    castle: false,
};

/// What unmaking a move needs that the move itself does not carry.
#[derive(Clone, Copy)]
struct Undo {
    ep: u8,
    castling: u8,
}

pub struct Chess {
    /// Positive for white, negative for black, `EDGE` off the board.
    board: [i8; BOARD],
    /// True when it is white's turn.
    white_to_move: bool,
    /// The square a pawn just skipped over, or 0.
    ep: u8,
    /// Castling rights: white king side, white queen side, then black's.
    castling: u8,
    pub phase: Phase,
    /// True when the human plays white.
    pub human_white: bool,
    pub think_index: usize,
    /// Square the human has picked up, if any.
    picked: Option<u8>,
    /// Legal destinations for the picked piece.
    hints: [u8; 32],
    n_hints: usize,
    /// Search state, kept between slices.
    started_ms: u32,
    deadline_ms: u32,
    depth: u8,
    best: Move,
    best_this_depth: Move,
    searching: bool,
    nodes: u32,
    /// The reply being played out, so the board shows it a moment after the
    /// machine has decided rather than in the same frame the finger lifted.
    show_at_ms: u32,
    outcome: Outcome,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Playing,
    WhiteMates,
    BlackMates,
    Stalemate,
}

impl Chess {
    pub const fn new() -> Self {
        Self {
            board: [EDGE; BOARD],
            white_to_move: true,
            ep: 0,
            castling: 0b1111,
            phase: Phase::Setup,
            human_white: true,
            think_index: 1,
            picked: None,
            hints: [0; 32],
            n_hints: 0,
            started_ms: 0,
            deadline_ms: 0,
            depth: 0,
            best: NO_MOVE,
            best_this_depth: NO_MOVE,
            searching: false,
            nodes: 0,
            show_at_ms: 0,
            outcome: Outcome::Playing,
        }
    }

    /// Back to the setup panel, keeping the chosen side and think time.
    pub fn restart(&mut self) {
        let (human_white, think_index) = (self.human_white, self.think_index);
        *self = Self::new();
        self.human_white = human_white;
        self.think_index = think_index;
    }

    pub fn setup(&mut self) {
        self.phase = Phase::Setup;
    }

    pub fn choose_side(&mut self, white: bool) {
        self.human_white = white;
    }

    pub fn cycle_think(&mut self) {
        self.think_index = (self.think_index + 1) % THINK_CHOICES.len();
    }

    pub fn think_seconds(&self) -> u16 {
        THINK_CHOICES[self.think_index.min(THINK_CHOICES.len() - 1)]
    }

    /// Lay out the pieces and decide who is on the clock.
    pub fn begin(&mut self, now_ms: u32) {
        const RANK: [i8; 8] = [ROOK, KNIGHT, BISHOP, QUEEN, KING, BISHOP, KNIGHT, ROOK];
        self.board = [EDGE; BOARD];
        for rank in 0..8 {
            for file in 0..8 {
                self.board[START + rank * 10 + file] = EMPTY;
            }
        }
        for file in 0..8 {
            self.board[START + file] = -RANK[file];
            self.board[START + 10 + file] = -PAWN;
            self.board[START + 60 + file] = PAWN;
            self.board[START + 70 + file] = RANK[file];
        }
        self.white_to_move = true;
        self.ep = 0;
        self.castling = 0b1111;
        self.picked = None;
        self.n_hints = 0;
        self.outcome = Outcome::Playing;
        self.searching = false;
        if self.human_white {
            self.phase = Phase::HumanTurn;
        } else {
            self.start_search(now_ms);
        }
    }

    /// Screen point to mailbox square, from the human's point of view: playing
    /// black turns the board round, which is what everyone expects.
    fn square_at(&self, x: i32, y: i32) -> Option<u8> {
        let file = (x - BOARD_X) / CELL;
        let rank = (y - BOARD_Y) / CELL;
        if !(0..8).contains(&file) || !(0..8).contains(&rank) {
            return None;
        }
        let (file, rank) = if self.human_white {
            (file, rank)
        } else {
            (7 - file, 7 - rank)
        };
        Some((START as i32 + rank * 10 + file) as u8)
    }

    fn screen_of(&self, square: u8) -> (i32, i32) {
        let index = square as i32 - START as i32;
        let (mut file, mut rank) = (index % 10, index / 10);
        if !self.human_white {
            file = 7 - file;
            rank = 7 - rank;
        }
        (
            BOARD_X + file * CELL + CELL / 2,
            BOARD_Y + rank * CELL + CELL / 2,
        )
    }

    pub fn tap(&mut self, x: i32, y: i32, now_ms: u32) {
        if self.phase != Phase::HumanTurn {
            return;
        }
        let Some(square) = self.square_at(x, y) else {
            return;
        };
        let piece = self.board[square as usize];
        let mine = if self.human_white { piece > 0 } else { piece < 0 };

        // Tapping one of your own pieces always picks it up, even mid-move, so a
        // mis-tap costs nothing.
        if mine && piece != EDGE {
            self.picked = Some(square);
            self.collect_hints(square);
            return;
        }
        let Some(from) = self.picked else {
            return;
        };
        let mut legal = [NO_MOVE; MAX_MOVES];
        let n = self.legal_moves(&mut legal);
        if let Some(chosen) = legal[..n]
            .iter()
            .copied()
            .find(|m| m.from == from && m.to == square)
        {
            let _ = self.make(chosen);
            self.picked = None;
            self.n_hints = 0;
            if self.finished() {
                self.phase = Phase::Done;
            } else {
                self.start_search(now_ms);
            }
        }
    }

    fn collect_hints(&mut self, from: u8) {
        let mut legal = [NO_MOVE; MAX_MOVES];
        let n = self.legal_moves(&mut legal);
        self.n_hints = 0;
        for m in legal[..n].iter().filter(|m| m.from == from) {
            if self.n_hints < self.hints.len() {
                self.hints[self.n_hints] = m.to;
                self.n_hints += 1;
            }
        }
    }

    fn start_search(&mut self, now_ms: u32) {
        self.phase = Phase::Thinking;
        self.started_ms = now_ms;
        self.deadline_ms = now_ms + self.think_seconds() as u32 * 1000;
        self.depth = 1;
        self.best = NO_MOVE;
        self.best_this_depth = NO_MOVE;
        self.searching = true;
        self.nodes = 0;
    }

    /// One slice of search, then the move once the clock is up. Returns true when
    /// the board changed and the screen needs repainting.
    pub fn think(&mut self, now_ms: u32) -> bool {
        if self.phase != Phase::Thinking {
            return false;
        }
        if !self.searching {
            // Decided; the move is shown after a short beat so it reads as a reply
            // rather than as part of the same instant.
            if now_ms.wrapping_sub(self.show_at_ms) < u32::MAX / 2 {
                let chosen = self.best;
                if chosen == NO_MOVE {
                    self.phase = Phase::Done;
                    self.outcome = self.decide_outcome();
                    return true;
                }
                let _ = self.make(chosen);
                self.phase = if self.finished() {
                    Phase::Done
                } else {
                    Phase::HumanTurn
                };
                return true;
            }
            return false;
        }

        let budget = self.nodes + NODES_PER_SLICE;
        let mut legal = [NO_MOVE; MAX_MOVES];
        let n = self.legal_moves(&mut legal);
        if n == 0 {
            self.searching = false;
            self.best = NO_MOVE;
            self.show_at_ms = now_ms;
            return false;
        }
        order(&mut legal[..n], &self.board);

        // One depth per slice group: the root is re-searched each time, which is
        // what iterative deepening does anyway, and it keeps all the state that
        // has to survive a return in one place - the depth and the best move.
        let mut alpha = -1_000_000;
        for index in 0..n {
            let m = legal[index];
            let undo = self.make(m);
            let score = -self.search(self.depth as i32 - 1, -1_000_000, -alpha, 1, budget);
            self.unmake(m, undo);
            if score > alpha {
                alpha = score;
                self.best_this_depth = m;
            }
            if self.nodes >= budget {
                // Out of nodes for this frame. The depth is not finished, so what
                // it found is not trusted - the next call starts it again.
                return false;
            }
        }
        self.best = self.best_this_depth;
        esp_println::println!(
            "chess: depth {} done, {} nodes in {} ms",
            self.depth,
            self.nodes,
            now_ms.wrapping_sub(self.started_ms)
        );
        self.depth += 1;
        // Deep enough, or out of time: eighteen ply is past anything this
        // evaluation can use and stops a mate-in-two search spinning.
        if now_ms.wrapping_sub(self.deadline_ms) < u32::MAX / 2 || self.depth > 18 {
            self.searching = false;
            self.show_at_ms = now_ms + 150;
        }
        false
    }

    fn search(&mut self, depth: i32, mut alpha: i32, beta: i32, ply: usize, budget: u32) -> i32 {
        self.nodes += 1;
        if depth <= 0 || ply >= MAX_PLY {
            return self.evaluate();
        }
        let mut legal = [NO_MOVE; MAX_MOVES];
        let n = self.legal_moves(&mut legal);
        if n == 0 {
            // Mate is worse the sooner it comes, so a forced mate is preferred to
            // a slow one and avoided as long as possible when it is ours.
            return if self.in_check(self.white_to_move) {
                -100_000 + ply as i32
            } else {
                0
            };
        }
        order(&mut legal[..n], &self.board);
        for index in 0..n {
            let m = legal[index];
            let undo = self.make(m);
            let score = -self.search(depth - 1, -beta, -alpha, ply + 1, budget);
            self.unmake(m, undo);
            if score >= beta {
                return beta;
            }
            if score > alpha {
                alpha = score;
            }
            if self.nodes >= budget {
                break;
            }
        }
        alpha
    }

    /// Material and a pull towards the centre, from the side to move's view.
    fn evaluate(&self) -> i32 {
        let mut score = 0;
        for rank in 0..8usize {
            for file in 0..8usize {
                let piece = self.board[START + rank * 10 + file];
                if piece == EMPTY || piece == EDGE {
                    continue;
                }
                let kind = piece.unsigned_abs() as usize;
                let mut value = VALUE[kind.min(6)];
                // Kings are not drawn towards the middle; everything else is.
                if kind != KING as usize {
                    value += (CENTRE[file] + CENTRE[rank]) / 2;
                }
                score += if piece > 0 { value } else { -value };
            }
        }
        if self.white_to_move { score } else { -score }
    }

    /// Pseudo-legal moves filtered by whether they leave the king attacked.
    fn legal_moves(&mut self, out: &mut [Move; MAX_MOVES]) -> usize {
        let mut pseudo = [NO_MOVE; MAX_MOVES];
        let count = self.generate(&mut pseudo);
        let mut n = 0;
        let side = self.white_to_move;
        for index in 0..count {
            let m = pseudo[index];
            let undo = self.make(m);
            let ok = !self.in_check(side);
            self.unmake(m, undo);
            if ok {
                out[n] = m;
                n += 1;
            }
        }
        n
    }

    fn generate(&self, out: &mut [Move; MAX_MOVES]) -> usize {
        let mut n = 0;
        let mut push = |m: Move, n: &mut usize| {
            if *n < MAX_MOVES {
                out[*n] = m;
                *n += 1;
            }
        };
        let white = self.white_to_move;
        for square in START..(START + 80) {
            let piece = self.board[square];
            if piece == EMPTY || piece == EDGE {
                continue;
            }
            if (piece > 0) != white {
                continue;
            }
            let kind = piece.unsigned_abs() as i8;
            if kind == PAWN {
                let step: i32 = if white { -10 } else { 10 };
                let start_rank = if white { 8 } else { 3 };
                let ahead = (square as i32 + step) as usize;
                if self.board[ahead] == EMPTY {
                    let last = if white { ahead < START + 10 } else { ahead >= START + 70 };
                    for kind in promotions(last) {
                        push(
                            Move {
                                from: square as u8,
                                to: ahead as u8,
                                promote: *kind,
                                ..NO_MOVE
                            },
                            &mut n,
                        );
                    }
                    let two = (square as i32 + step * 2) as usize;
                    if square / 10 == start_rank && self.board[two] == EMPTY {
                        push(
                            Move {
                                from: square as u8,
                                to: two as u8,
                                ..NO_MOVE
                            },
                            &mut n,
                        );
                    }
                }
                for side_step in [step - 1, step + 1] {
                    let target = (square as i32 + side_step) as usize;
                    let occupant = self.board[target];
                    if occupant == EDGE {
                        continue;
                    }
                    let enemy = occupant != EMPTY && (occupant > 0) != white;
                    if enemy {
                        let last = if white {
                            target < START + 10
                        } else {
                            target >= START + 70
                        };
                        for kind in promotions(last) {
                            push(
                                Move {
                                    from: square as u8,
                                    to: target as u8,
                                    taken: occupant,
                                    promote: *kind,
                                    ..NO_MOVE
                                },
                                &mut n,
                            );
                        }
                    } else if occupant == EMPTY && target as u8 == self.ep && self.ep != 0 {
                        let grabbed = (target as i32 - step) as usize;
                        push(
                            Move {
                                from: square as u8,
                                to: target as u8,
                                taken: self.board[grabbed],
                                en_passant: true,
                                ..NO_MOVE
                            },
                            &mut n,
                        );
                    }
                }
                continue;
            }

            let kinds = kind.clamp(0, 6) as usize;
            for slot in 0..N_OFFSETS[kinds] {
                let step = OFFSETS[kinds][slot] as i32;
                let mut target = square as i32;
                loop {
                    target += step;
                    let occupant = self.board[target as usize];
                    if occupant == EDGE {
                        break;
                    }
                    if occupant != EMPTY {
                        if (occupant > 0) != white {
                            push(
                                Move {
                                    from: square as u8,
                                    to: target as u8,
                                    taken: occupant,
                                    ..NO_MOVE
                                },
                                &mut n,
                            );
                        }
                        break;
                    }
                    push(
                        Move {
                            from: square as u8,
                            to: target as u8,
                            ..NO_MOVE
                        },
                        &mut n,
                    );
                    if !SLIDES[kinds] {
                        break;
                    }
                }
            }

            if kind == KING {
                // Castling: rights, an empty path, and no square the king crosses
                // may be attacked - including the one it starts on.
                let (home, king_side, queen_side) = if white {
                    (95usize, 0b0001, 0b0010)
                } else {
                    (25usize, 0b0100, 0b1000)
                };
                if square == home && !self.in_check(white) {
                    if self.castling & king_side != 0
                        && self.board[home + 1] == EMPTY
                        && self.board[home + 2] == EMPTY
                        && !self.attacked(home + 1, !white)
                    {
                        push(
                            Move {
                                from: home as u8,
                                to: (home + 2) as u8,
                                castle: true,
                                ..NO_MOVE
                            },
                            &mut n,
                        );
                    }
                    if self.castling & queen_side != 0
                        && self.board[home - 1] == EMPTY
                        && self.board[home - 2] == EMPTY
                        && self.board[home - 3] == EMPTY
                        && !self.attacked(home - 1, !white)
                    {
                        push(
                            Move {
                                from: home as u8,
                                to: (home - 2) as u8,
                                castle: true,
                                ..NO_MOVE
                            },
                            &mut n,
                        );
                    }
                }
            }
        }
        n
    }

    fn make(&mut self, m: Move) -> Undo {
        let undo = Undo {
            ep: self.ep,
            castling: self.castling,
        };
        let from = m.from as usize;
        let to = m.to as usize;
        let piece = self.board[from];
        let white = piece > 0;
        self.board[from] = EMPTY;
        self.board[to] = if m.promote != EMPTY {
            if white { m.promote } else { -m.promote }
        } else {
            piece
        };
        if m.en_passant {
            let grabbed = if white { to + 10 } else { to - 10 };
            self.board[grabbed] = EMPTY;
        }
        if m.castle {
            // The rook hops to the square the king passed over.
            let (rook_from, rook_to) = if to > from {
                (to + 1, to - 1)
            } else {
                (to - 2, to + 1)
            };
            self.board[rook_to] = self.board[rook_from];
            self.board[rook_from] = EMPTY;
        }

        // A two-square pawn push is the only thing that offers en passant.
        self.ep = 0;
        if piece.abs() == PAWN && from.abs_diff(to) == 20 {
            self.ep = ((from + to) / 2) as u8;
        }
        // Rights are lost by moving the king or a rook, or by the rook's square
        // being captured on.
        for (square, mask) in [(95usize, 0b0011u8), (25, 0b1100)] {
            if from == square {
                self.castling &= !mask;
            }
        }
        for (square, mask) in [
            (98usize, 0b0001u8),
            (91, 0b0010),
            (28, 0b0100),
            (21, 0b1000),
        ] {
            if from == square || to == square {
                self.castling &= !mask;
            }
        }
        self.white_to_move = !self.white_to_move;
        undo
    }

    fn unmake(&mut self, m: Move, undo: Undo) {
        let from = m.from as usize;
        let to = m.to as usize;
        let piece = self.board[to];
        let white = piece > 0;
        self.board[from] = if m.promote != EMPTY {
            if white { PAWN } else { -PAWN }
        } else {
            piece
        };
        self.board[to] = if m.en_passant { EMPTY } else { m.taken };
        if m.en_passant {
            let grabbed = if white { to + 10 } else { to - 10 };
            self.board[grabbed] = m.taken;
        }
        if m.castle {
            let (rook_from, rook_to) = if to > from {
                (to + 1, to - 1)
            } else {
                (to - 2, to + 1)
            };
            self.board[rook_from] = self.board[rook_to];
            self.board[rook_to] = EMPTY;
        }
        self.ep = undo.ep;
        self.castling = undo.castling;
        self.white_to_move = !self.white_to_move;
    }

    fn in_check(&self, white: bool) -> bool {
        let king = if white { KING } else { -KING };
        let Some(square) = (START..START + 80).find(|&s| self.board[s] == king) else {
            return false;
        };
        self.attacked(square, !white)
    }

    /// Whether `square` is attacked by the side `by_white`. Runs outward from the
    /// square rather than over every enemy piece, which is the cheaper direction.
    fn attacked(&self, square: usize, by_white: bool) -> bool {
        let sign: i8 = if by_white { 1 } else { -1 };
        for step in OFFSETS[KNIGHT as usize] {
            let target = square as i32 + step as i32;
            if self.board[target as usize] == KNIGHT * sign {
                return true;
            }
        }
        // Pawns attack towards the defender, so the step is reversed here.
        let pawn_step: i32 = if by_white { 10 } else { -10 };
        for side in [-1, 1] {
            let target = square as i32 + pawn_step + side;
            if self.board[target as usize] == PAWN * sign {
                return true;
            }
        }
        for (index, step) in OFFSETS[KING as usize].into_iter().enumerate() {
            let diagonal = matches!(index, 0 | 2 | 5 | 7);
            let mut target = square as i32;
            let mut distance = 0;
            loop {
                target += step as i32;
                distance += 1;
                let occupant = self.board[target as usize];
                if occupant == EDGE {
                    break;
                }
                if occupant == EMPTY {
                    continue;
                }
                if occupant / sign > 0 {
                    let kind = occupant.abs();
                    let hits = kind == QUEEN
                        || (distance == 1 && kind == KING)
                        || (diagonal && kind == BISHOP)
                        || (!diagonal && kind == ROOK);
                    if hits {
                        return true;
                    }
                }
                break;
            }
        }
        false
    }

    fn finished(&mut self) -> bool {
        let mut legal = [NO_MOVE; MAX_MOVES];
        if self.legal_moves(&mut legal) > 0 {
            return false;
        }
        self.outcome = self.decide_outcome();
        true
    }

    fn decide_outcome(&self) -> Outcome {
        if !self.in_check(self.white_to_move) {
            Outcome::Stalemate
        } else if self.white_to_move {
            Outcome::BlackMates
        } else {
            Outcome::WhiteMates
        }
    }

    pub fn draw(&self, scene: &mut Scene, now_ms: u32, alpha: u8) {
        if self.phase == Phase::Setup {
            self.draw_setup(scene, alpha);
            return;
        }

        // The light squares are one rectangle; only the dark ones are drawn, which
        // halves what the board costs in primitives.
        scene.pill(
            BOARD_X,
            BOARD_Y,
            BOARD_X + CELL * 8,
            BOARD_Y + CELL * 8,
            4,
            LIGHT,
            alpha,
        );
        for rank in 0..8 {
            for file in 0..8 {
                if (rank + file) % 2 == 0 {
                    continue;
                }
                let x = BOARD_X + file * CELL;
                let y = BOARD_Y + rank * CELL;
                scene.pill(x, y, x + CELL, y + CELL, 0, DARK, alpha);
            }
        }

        if let Some(square) = self.picked {
            let (x, y) = self.screen_of(square);
            scene.ring(x, y, CELL / 2 - 2, CELL / 2 - 7, PICK, alpha);
        }
        for hint in self.hints[..self.n_hints].iter() {
            let (x, y) = self.screen_of(*hint);
            scene.disc(x, y, 7, HINT, alpha);
        }

        for rank in 0..8usize {
            for file in 0..8usize {
                let piece = self.board[START + rank * 10 + file];
                if piece == EMPTY || piece == EDGE {
                    continue;
                }
                let square = (START + rank * 10 + file) as u8;
                let (x, y) = self.screen_of(square);
                let white = piece > 0;
                scene.disc(
                    x,
                    y,
                    CELL / 2 - 6,
                    if white { WHITE_PIECE } else { BLACK_PIECE },
                    alpha,
                );
                scene.label(
                    x,
                    y + 9,
                    FontId::Caption,
                    if white { BLACK_PIECE } else { WHITE_PIECE },
                    alpha,
                    Align::Center,
                    letter(piece.abs()),
                );
            }
        }

        let mut status = TextBuf::new();
        match (self.phase, self.outcome) {
            (Phase::Done, Outcome::Stalemate) => {
                let _ = write!(status, "STALEMATE - TAP TO PLAY");
            }
            (Phase::Done, outcome) => {
                let won = (outcome == Outcome::WhiteMates) == self.human_white;
                let _ = write!(
                    status,
                    "{} - TAP TO PLAY",
                    if won { "YOU WIN" } else { "CHECKMATE" }
                );
            }
            (Phase::Thinking, _) => {
                // The dots move so a twenty-second think does not look like a hang.
                let dots = (now_ms / 400 % 4) as usize;
                let _ = write!(status, "THINKING{}", &"..."[..dots.min(3)]);
            }
            _ => {
                let check = self.in_check(self.white_to_move);
                let _ = write!(status, "{}", if check { "CHECK" } else { "YOUR MOVE" });
            }
        }
        scene.label(
            W as i32 / 2,
            STATUS_Y,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            status.as_str(),
        );
    }

    fn draw_setup(&self, scene: &mut Scene, alpha: u8) {
        scene.label(
            W as i32 / 2,
            96,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            "CHESS",
        );
        for (index, rect) in SETUP.iter().enumerate() {
            let (x0, y0, x1, y1) = *rect;
            let chosen = match index {
                0 => self.human_white,
                1 => !self.human_white,
                _ => false,
            };
            let color = match index {
                0 => WHITE_PIECE,
                1 => rgb(90, 96, 108),
                2 => ACCENT,
                _ => HINT,
            };
            let r = (y1 - y0) / 2;
            scene.pill(
                x0,
                y0,
                x1,
                y1,
                r,
                if chosen || index >= 2 {
                    color
                } else {
                    rgb(24, 28, 34)
                },
                alpha,
            );
            let mut caption = TextBuf::new();
            let _ = match index {
                0 => write!(caption, "YOU PLAY WHITE"),
                1 => write!(caption, "YOU PLAY BLACK"),
                2 => write!(caption, "THINKS {} S", self.think_seconds()),
                _ => write!(caption, "START"),
            };
            scene.label(
                (x0 + x1) / 2,
                (y0 + y1) / 2 + 13,
                FontId::Body,
                if chosen || index >= 2 {
                    rgb(12, 14, 18)
                } else {
                    muted(color)
                },
                alpha,
                Align::Center,
                caption.as_str(),
            );
        }
        scene.label(
            W as i32 / 2,
            H as i32 - 22,
            FontId::Micro,
            rgb(140, 152, 166),
            alpha,
            Align::Center,
            "TAP THE TIME TO CHANGE IT",
        );
    }
}

/// The setup panel's four controls: two sides, the think time, and start.
pub const SETUP: [(i32, i32, i32, i32); 4] = [
    (60, 140, 420, 200),
    (60, 212, 420, 272),
    (60, 284, 420, 344),
    (60, 366, 420, 434),
];

pub fn setup_at(x: i32, y: i32) -> Option<usize> {
    SETUP.iter().position(|(x0, y0, x1, y1)| {
        x >= *x0 && x <= *x1 && y >= *y0 && y <= *y1
    })
}

fn letter(kind: i8) -> &'static str {
    match kind {
        1 => "P",
        2 => "N",
        3 => "B",
        4 => "R",
        5 => "Q",
        _ => "K",
    }
}

/// Captures first, by what they take. Cheap, and it is most of what alpha-beta
/// needs to prune well - without it the search is several plies shallower for the
/// same time.
fn order(moves: &mut [Move], board: &[i8; BOARD]) {
    moves.sort_unstable_by_key(|m| {
        let mut gain = if m.taken == EMPTY {
            0
        } else {
            VALUE[m.taken.unsigned_abs() as usize]
        };
        if m.promote != EMPTY {
            gain += VALUE[m.promote.unsigned_abs() as usize];
        }
        let mover = VALUE[board[m.from as usize].unsigned_abs() as usize];
        // Taking a big piece with a small one first.
        -(gain * 16 - mover)
    });
}

/// The pieces a pawn may become on the last rank, queen first so the human's tap
/// resolves to one; a single EMPTY when it is not promoting at all.
fn promotions(last: bool) -> &'static [i8] {
    if last {
        &[QUEEN, ROOK, BISHOP, KNIGHT]
    } else {
        &[EMPTY]
    }
}
