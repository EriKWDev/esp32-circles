//! Chess against the panel.
//!
//! The board is a 10x12 mailbox rather than a bare 8x8: the two-square border
//! makes every off-board test a single array lookup, which is what keeps knight
//! and sliding generation free of edge arithmetic and the bugs that come with it.
//!
//! The search is negamax with alpha-beta and iterative deepening, with a
//! quiescence search at the leaves, killer moves and the previous iteration's best
//! move driving the ordering. Quiescence is the one that matters most: without it
//! a leaf can fall in the middle of a capture exchange, and the engine gives away
//! a piece for nothing at the horizon. It plays out captures until the position is
//! quiet before believing what it is looking at.
//!
//! Openings come from a small book of main lines rather than from search. Twenty
//! seconds of this evaluation does not find the Ruy Lopez, and every book move is
//! checked against the generator before it can be played.
//!
//! It runs in slices. It has to: the think time goes up to twenty seconds and this is the
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
const NODES_PER_SLICE: u32 = 3_000;
/// Ceiling on a doubled slice. The doubling is what guarantees a root move
/// eventually completes; the ceiling only stops it growing without bound.
const MAX_SLICE: u32 = 60_000;
/// How much more a depth costs than the one before it, roughly, with this ordering.
/// Used to decide whether the next one is worth starting: a depth that will not
/// finish is time spent for nothing, since only a completed depth is trusted.
const DEPTH_COST_FACTOR: u32 = 4;
/// Captures are followed this deep past the search horizon and no further. A
/// capture sequence can be long, and an unbounded one makes the cost of a single
/// leaf unpredictable, which is exactly what the slice budget cannot tolerate.
const MAX_QUIESCE: usize = 6;
/// Past this the evaluation has nothing more to give and the wait is not repaid.
const MAX_DEPTH: u8 = 6;

const VALUE: [i32; 7] = [0, 100, 320, 330, 500, 900, 20_000];

/// Where each piece wants to stand, in centipawns, written from white's side with
/// rank 8 first. Black reads the same tables mirrored. This replaces a generic pull
/// towards the centre, which valued a knight on the rim the same as a rook there
/// and gave the machine no reason to castle or to push a pawn.
const PST: [[i8; 64]; 7] = [
    [0; 64],
    // Pawn: advance, but not the ones in front of a castled king.
    [
        0, 0, 0, 0, 0, 0, 0, 0, //
        50, 50, 50, 50, 50, 50, 50, 50, //
        10, 10, 20, 30, 30, 20, 10, 10, //
        5, 5, 10, 25, 25, 10, 5, 5, //
        0, 0, 0, 20, 20, 0, 0, 0, //
        5, -5, -10, 0, 0, -10, -5, 5, //
        5, 10, 10, -20, -20, 10, 10, 5, //
        0, 0, 0, 0, 0, 0, 0, 0,
    ],
    // Knight: the rim is poison.
    [
        -50, -40, -30, -30, -30, -30, -40, -50, //
        -40, -20, 0, 0, 0, 0, -20, -40, //
        -30, 0, 10, 15, 15, 10, 0, -30, //
        -30, 5, 15, 20, 20, 15, 5, -30, //
        -30, 0, 15, 20, 20, 15, 0, -30, //
        -30, 5, 10, 15, 15, 10, 5, -30, //
        -40, -20, 0, 5, 5, 0, -20, -40, //
        -50, -40, -30, -30, -30, -30, -40, -50,
    ],
    // Bishop: long diagonals.
    [
        -20, -10, -10, -10, -10, -10, -10, -20, //
        -10, 0, 0, 0, 0, 0, 0, -10, //
        -10, 0, 5, 10, 10, 5, 0, -10, //
        -10, 5, 5, 10, 10, 5, 5, -10, //
        -10, 0, 10, 10, 10, 10, 0, -10, //
        -10, 10, 10, 10, 10, 10, 10, -10, //
        -10, 5, 0, 0, 0, 0, 5, -10, //
        -20, -10, -10, -10, -10, -10, -10, -20,
    ],
    // Rook: the seventh rank, and the centre files.
    [
        0, 0, 0, 0, 0, 0, 0, 0, //
        5, 10, 10, 10, 10, 10, 10, 5, //
        -5, 0, 0, 0, 0, 0, 0, -5, //
        -5, 0, 0, 0, 0, 0, 0, -5, //
        -5, 0, 0, 0, 0, 0, 0, -5, //
        -5, 0, 0, 0, 0, 0, 0, -5, //
        -5, 0, 0, 0, 0, 0, 0, -5, //
        0, 0, 0, 5, 5, 0, 0, 0,
    ],
    // Queen: no early adventures.
    [
        -20, -10, -10, -5, -5, -10, -10, -20, //
        -10, 0, 0, 0, 0, 0, 0, -10, //
        -10, 0, 5, 5, 5, 5, 0, -10, //
        -5, 0, 5, 5, 5, 5, 0, -5, //
        0, 0, 5, 5, 5, 5, 0, -5, //
        -10, 5, 5, 5, 5, 5, 0, -10, //
        -10, 0, 5, 0, 0, 0, 0, -10, //
        -20, -10, -10, -5, -5, -10, -10, -20,
    ],
    // King, while there are still pieces about: behind the pawns, castled.
    [
        -30, -40, -40, -50, -50, -40, -40, -30, //
        -30, -40, -40, -50, -50, -40, -40, -30, //
        -30, -40, -40, -50, -50, -40, -40, -30, //
        -30, -40, -40, -50, -50, -40, -40, -30, //
        -20, -30, -30, -40, -40, -30, -30, -20, //
        -10, -20, -20, -20, -20, -20, -20, -10, //
        20, 20, 0, 0, 0, 0, 20, 20, //
        20, 30, 10, 0, 0, 10, 30, 20,
    ],
];

/// The king wants the opposite thing once the board empties, so it has a second
/// table - a king that hides in the corner in a pawn endgame loses it.
const KING_END: [i8; 64] = [
    -50, -40, -30, -20, -20, -30, -40, -50, //
    -30, -20, -10, 0, 0, -10, -20, -30, //
    -30, -10, 20, 30, 30, 20, -10, -30, //
    -30, -10, 30, 40, 40, 30, -10, -30, //
    -30, -10, 30, 40, 40, 30, -10, -30, //
    -30, -10, 20, 30, 30, 20, -10, -30, //
    -30, -30, 0, 0, 0, 0, -30, -30, //
    -50, -30, -30, -30, -30, -30, -30, -50,
];

/// Non-pawn material below which the endgame king table takes over.
const ENDGAME_MATERIAL: i32 = 1_300;

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
    /// Whether there is a game to go back to. The pieces are left standing when
    /// the page is left, so leaving it is not resigning.
    pub has_game: bool,
    /// Square the human has picked up, if any.
    picked: Option<u8>,
    /// Legal destinations for the picked piece.
    hints: [u8; 32],
    n_hints: usize,
    /// Search state, kept between slices.
    started_ms: u32,
    /// When the depth being searched began, so the next one can be costed before it
    /// is started.
    depth_started_ms: u32,
    deadline_ms: u32,
    depth: u8,
    best: Move,
    best_this_depth: Move,
    searching: bool,
    nodes: u32,
    /// How far through the root move list this depth has got.
    root_index: usize,
    root_alpha: i32,
    /// Nodes this slice may spend. Doubles whenever one root move needs more than
    /// a whole slice, so progress is guaranteed at any depth.
    slice_budget: u32,
    /// Set deep in the search when the budget runs out, unwinding it at once.
    aborted: bool,
    /// Two quiet moves per ply that have caused a cutoff before.
    killers: [[Move; 2]; MAX_PLY],
    /// The game so far, for matching against the opening book.
    history: [(u8, u8); 64],
    n_history: usize,
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
            has_game: false,
            picked: None,
            hints: [0; 32],
            n_hints: 0,
            started_ms: 0,
            depth_started_ms: 0,
            deadline_ms: 0,
            depth: 0,
            best: NO_MOVE,
            best_this_depth: NO_MOVE,
            searching: false,
            nodes: 0,
            root_index: 0,
            root_alpha: -1_000_000,
            slice_budget: NODES_PER_SLICE,
            aborted: false,
            killers: [[NO_MOVE; 2]; MAX_PLY],
            history: [(0, 0); 64],
            n_history: 0,
            show_at_ms: 0,
            outcome: Outcome::Playing,
        }
    }

    /// Show the setup panel without disturbing the board, so a game survives a
    /// trip to the schedules and back.
    pub fn setup(&mut self) {
        self.phase = Phase::Setup;
    }

    /// Choosing a side abandons whatever was on the board: the two are the same
    /// decision, since a side cannot be swapped mid-game.
    pub fn choose_side(&mut self, white: bool) {
        self.human_white = white;
        self.has_game = false;
        self.searching = false;
    }

    /// Pick the game back up. The machine takes over again if it was its move.
    pub fn resume(&mut self, now_ms: u32) {
        if !self.has_game {
            return;
        }
        if self.white_to_move == self.human_white {
            self.phase = Phase::HumanTurn;
        } else {
            self.start_search(now_ms);
        }
    }

    pub fn cycle_think(&mut self) {
        self.think_index = (self.think_index + 1) % THINK_CHOICES.len();
    }

    /// True while the machine is on the clock, so the page can animate.
    pub fn thinking(&self) -> bool {
        self.phase == Phase::Thinking
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
        self.has_game = true;
        self.n_history = 0;
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
            self.record(chosen);
            let _ = self.make(chosen);
            self.picked = None;
            self.n_hints = 0;
            if self.finished() {
                self.has_game = false;
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

    /// A reply from the book, if the game so far is still in one of its lines.
    ///
    /// Every candidate is checked against the generator. The book is written by
    /// hand, so a typo is a question of when rather than whether, and an illegal
    /// move would be far worse than no book at all.
    fn book_move(&mut self, now_ms: u32) -> Option<Move> {
        let mut legal = [NO_MOVE; MAX_MOVES];
        let n = self.legal_moves(&mut legal);
        let mut candidates = [NO_MOVE; 8];
        let mut count = 0;
        for line in BOOK {
            let mut moves = line.split(' ');
            // Follow the line as far as the game has gone; it stays a candidate
            // only while every ply matches.
            let mut matched = true;
            for played in 0..self.n_history {
                match moves.next().and_then(parse_move) {
                    Some(step) if step == self.history[played] => {}
                    _ => {
                        matched = false;
                        break;
                    }
                }
            }
            if !matched {
                continue;
            }
            let Some((from, to)) = moves.next().and_then(parse_move) else {
                continue;
            };
            let Some(found) = legal[..n]
                .iter()
                .copied()
                .find(|m| m.from == from && m.to == to)
            else {
                continue;
            };
            if !candidates[..count].contains(&found) && count < candidates.len() {
                candidates[count] = found;
                count += 1;
            }
        }
        if count == 0 {
            return None;
        }
        // Any of the matching lines will do, and varying keeps successive games
        // from being identical.
        Some(candidates[(now_ms as usize / 7) % count])
    }

    fn start_search(&mut self, now_ms: u32) {
        self.phase = Phase::Thinking;
        if let Some(m) = self.book_move(now_ms) {
            // Straight to the played-out state: there is nothing to search, but the
            // move still arrives after the same short beat as any other.
            self.best = m;
            self.searching = false;
            self.show_at_ms = now_ms + 250;
            esp_println::println!("chess: book move");
            return;
        }
        self.started_ms = now_ms;
        self.depth_started_ms = now_ms;
        self.deadline_ms = now_ms + self.think_seconds() as u32 * 1000;
        self.depth = 1;
        self.best = NO_MOVE;
        self.best_this_depth = NO_MOVE;
        self.searching = true;
        self.nodes = 0;
        self.root_index = 0;
        self.root_alpha = -1_000_000;
        self.slice_budget = NODES_PER_SLICE;
        self.aborted = false;
        self.killers = [[NO_MOVE; 2]; MAX_PLY];
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
                self.record(chosen);
                let _ = self.make(chosen);
                self.phase = if self.finished() {
                    self.has_game = false;
                    Phase::Done
                } else {
                    Phase::HumanTurn
                };
                return true;
            }
            return false;
        }

        // The clock is checked here, once per frame, and not only where a depth
        // finishes. That was the bug behind thinking for ever: a depth too large
        // for one slice never completed, so the only time check in the code was
        // never reached. It also makes the setting mean what it says - a maximum,
        // not a duration.
        if self.out_of_time(now_ms) && self.best != NO_MOVE {
            self.stop_searching(now_ms);
            return false;
        }

        let mut legal = [NO_MOVE; MAX_MOVES];
        let n = self.legal_moves(&mut legal);
        if n == 0 {
            self.searching = false;
            self.best = NO_MOVE;
            self.show_at_ms = now_ms;
            return false;
        }
        order(&mut legal[..n], &self.board, &[NO_MOVE; 2]);
        // The previous depth's answer goes first: it is the most likely to be best
        // again, and a good first move is what makes everything after it cheap.
        if self.best != NO_MOVE
            && let Some(at) = legal[..n].iter().position(|m| *m == self.best)
        {
            legal[..=at].rotate_right(1);
        }
        if self.root_index >= n {
            self.root_index = 0;
        }

        // Where the root got to survives the return, so a slice continues the depth
        // it was in the middle of rather than starting it again. Throwing that work
        // away was the other half of the problem: nothing past depth two could ever
        // finish.
        let budget = self.nodes + self.slice_budget;
        while self.root_index < n {
            let m = legal[self.root_index];
            let undo = self.make(m);
            self.aborted = false;
            let score = -self.search(
                self.depth as i32 - 1,
                -1_000_000,
                -self.root_alpha,
                1,
                budget,
            );
            self.unmake(m, undo);
            if self.aborted {
                // This subtree did not fit in what was left of the slice. Its score
                // is meaningless, so it is not recorded and the root does not
                // advance - the next attempt gets a larger slice.
                self.slice_budget = (self.slice_budget * 2).min(MAX_SLICE);
                return false;
            }
            if score > self.root_alpha {
                self.root_alpha = score;
                self.best_this_depth = m;
            }
            self.root_index += 1;
            if self.nodes >= budget {
                return false;
            }
        }

        // A finished depth is the only kind whose answer is trusted.
        self.best = self.best_this_depth;
        esp_println::println!(
            "chess: depth {} = {} cp, {} nodes, {} ms",
            self.depth,
            self.root_alpha,
            self.nodes,
            now_ms.wrapping_sub(self.started_ms)
        );
        let mate = self.root_alpha > 90_000;
        let spent = now_ms.wrapping_sub(self.depth_started_ms);
        self.depth += 1;
        self.depth_started_ms = now_ms;
        self.root_index = 0;
        self.root_alpha = -1_000_000;
        self.best_this_depth = NO_MOVE;
        self.slice_budget = NODES_PER_SLICE;
        // Starting a depth that cannot finish is time spent for nothing, since only
        // a completed depth is trusted. The next one costs several times this one,
        // so if that will not fit, stop here and play what this depth chose. This is
        // also what makes the time setting mean something: a longer budget affords
        // another doubling, and another ply.
        let next_needs = spent.saturating_mul(DEPTH_COST_FACTOR).max(2);
        let left = self.deadline_ms.wrapping_sub(now_ms);
        let no_time_for_more = self.out_of_time(now_ms) || left < next_needs;
        if mate || self.depth > MAX_DEPTH || no_time_for_more {
            self.stop_searching(now_ms);
        }
        false
    }

    fn out_of_time(&self, now_ms: u32) -> bool {
        now_ms.wrapping_sub(self.deadline_ms) < u32::MAX / 2
    }

    fn stop_searching(&mut self, now_ms: u32) {
        self.searching = false;
        // A short beat before the piece moves, so it reads as a reply rather than
        // part of the same instant the finger lifted.
        self.show_at_ms = now_ms + 150;
    }

    fn search(&mut self, depth: i32, mut alpha: i32, beta: i32, ply: usize, budget: u32) -> i32 {
        self.nodes += 1;
        if self.aborted {
            return alpha;
        }
        if self.nodes >= budget {
            self.aborted = true;
            return alpha;
        }
        // A check is never a quiet position, so the search is extended rather than
        // handed to quiescence - otherwise a forced sequence gets cut off exactly
        // where it matters.
        let checked = self.in_check(self.white_to_move);
        let depth = if checked && depth < 4 { depth + 1 } else { depth };
        if depth <= 0 || ply >= MAX_PLY {
            return self.quiesce(alpha, beta, ply, budget, MAX_QUIESCE);
        }
        let mut legal = [NO_MOVE; MAX_MOVES];
        let n = self.legal_moves(&mut legal);
        if n == 0 {
            // Mate is worse the sooner it comes, so a forced mate is preferred to
            // a slow one and avoided as long as possible when it is ours.
            return if checked { -100_000 + ply as i32 } else { 0 };
        }
        order(&mut legal[..n], &self.board, &self.killers[ply.min(MAX_PLY - 1)]);
        for index in 0..n {
            let m = legal[index];
            let undo = self.make(m);
            let score = -self.search(depth - 1, -beta, -alpha, ply + 1, budget);
            self.unmake(m, undo);
            if self.aborted {
                return alpha;
            }
            if score >= beta {
                // A quiet move good enough to cut is worth trying first in its
                // sibling positions, which is most of what ordering can learn
                // without a table of positions to remember.
                if m.taken == EMPTY {
                    let slot = ply.min(MAX_PLY - 1);
                    self.killers[slot][1] = self.killers[slot][0];
                    self.killers[slot][0] = m;
                }
                return beta;
            }
            if score > alpha {
                alpha = score;
            }
        }
        alpha
    }

    /// Material and placement, from the side to move's view.
    fn evaluate(&self) -> i32 {
        // Two passes: the king tables depend on how much is left on the board, and
        // that is not known until everything has been counted.
        let mut heavy = 0;
        for square in START..START + 80 {
            let piece = self.board[square];
            if piece == EMPTY || piece == EDGE {
                continue;
            }
            let kind = piece.unsigned_abs() as usize;
            if kind != PAWN as usize && kind != KING as usize {
                heavy += VALUE[kind.min(6)];
            }
        }
        let endgame = heavy < ENDGAME_MATERIAL;

        let mut score = 0;
        for rank in 0..8usize {
            for file in 0..8usize {
                let piece = self.board[START + rank * 10 + file];
                if piece == EMPTY || piece == EDGE {
                    continue;
                }
                let kind = piece.unsigned_abs() as usize;
                // White reads the tables as written; black reads them mirrored.
                let index = if piece > 0 {
                    rank * 8 + file
                } else {
                    (7 - rank) * 8 + file
                };
                let placement = if kind == KING as usize && endgame {
                    KING_END[index]
                } else {
                    PST[kind.min(6)][index]
                } as i32;
                let value = VALUE[kind.min(6)] + placement;
                score += if piece > 0 { value } else { -value };
            }
        }
        if self.white_to_move { score } else { -score }
    }

    /// Captures and promotions only, played out until nothing is hanging.
    ///
    /// This is what stops the engine believing a position it has caught halfway
    /// through an exchange. Standing pat first means a side is never forced to
    /// capture when sitting still is better.
    fn quiesce(&mut self, mut alpha: i32, beta: i32, ply: usize, budget: u32, left: usize) -> i32 {
        self.nodes += 1;
        if self.aborted {
            return alpha;
        }
        if self.nodes >= budget {
            self.aborted = true;
            return alpha;
        }
        let stand_pat = self.evaluate();
        if stand_pat >= beta {
            return beta;
        }
        if stand_pat > alpha {
            alpha = stand_pat;
        }
        if ply >= MAX_PLY || left == 0 {
            return alpha;
        }

        let mut legal = [NO_MOVE; MAX_MOVES];
        let n = self.legal_moves(&mut legal);
        let mut loud = [NO_MOVE; MAX_MOVES];
        let mut count = 0;
        for m in legal[..n].iter() {
            if m.taken != EMPTY || m.promote != EMPTY {
                loud[count] = *m;
                count += 1;
            }
        }
        order(&mut loud[..count], &self.board, &[NO_MOVE; 2]);
        for index in 0..count {
            let m = loud[index];
            let undo = self.make(m);
            let score = -self.quiesce(-beta, -alpha, ply + 1, budget, left - 1);
            self.unmake(m, undo);
            if self.aborted {
                return alpha;
            }
            if score >= beta {
                return beta;
            }
            if score > alpha {
                alpha = score;
            }
        }
        alpha
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

    /// Records a move as played, for the book to match against. Only the moves
    /// actually played reach this - the search makes and unmakes far too many.
    fn record(&mut self, m: Move) {
        if self.n_history < self.history.len() {
            self.history[self.n_history] = (m.from, m.to);
            self.n_history += 1;
        }
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
            // Nothing to continue, nothing drawn - an inert button is worse than
            // an absent one.
            if index == CONTINUE && !self.has_game {
                continue;
            }
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
                CONTINUE => HINT,
                _ => rgb(120, 170, 255),
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
                2 => write!(caption, "THINKS {} S MAX", self.think_seconds()),
                CONTINUE => write!(caption, "CONTINUE"),
                _ => write!(caption, "NEW GAME"),
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
            "PICKING A SIDE STARTS OVER",
        );
    }
}

/// The setup panel: two sides, the think time, continue, and a new game.
pub const SETUP: [(i32, i32, i32, i32); 5] = [
    (60, 128, 420, 184),
    (60, 190, 420, 246),
    (60, 252, 420, 308),
    (60, 320, 420, 376),
    (60, 382, 420, 438),
];
/// Continue is the only one the tap handler names; the rest fall out in order.
pub const CONTINUE: usize = 3;

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

/// Captures first, by what they take, then killers. Cheap, and it is most of what
/// alpha-beta needs to prune well - without it the search is several plies
/// shallower for the same time.
fn order(moves: &mut [Move], board: &[i8; BOARD], killers: &[Move; 2]) {
    moves.sort_unstable_by_key(|m| {
        if killers[0] == *m {
            return -1_000_000;
        }
        if killers[1] == *m {
            return -900_000;
        }
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

/// Main lines, as plies in plain coordinate notation.
///
/// Kept as text because that is the form these can be checked in - by eye against
/// any opening reference, and by the generator, which is asked whether a book move
/// is legal before it is ever played. A line that turned out to be nonsense would
/// otherwise be unanswerable: the machine would simply make an impossible move.
static BOOK: [&str; 24] = [
    // King's pawn: Italian, Ruy Lopez, Scotch, Four Knights, Petrov, Vienna.
    "e2e4 e7e5 g1f3 b8c6 f1c4 g8f6 d2d3 f8c5 c2c3 d7d6",
    "e2e4 e7e5 g1f3 b8c6 f1c4 f8c5 c2c3 g8f6 d2d3 d7d6",
    "e2e4 e7e5 g1f3 b8c6 f1b5 a7a6 b5a4 g8f6 e1g1 f8e7",
    "e2e4 e7e5 g1f3 b8c6 f1b5 g8f6 e1g1 f6e4 d2d4 e4d6",
    "e2e4 e7e5 g1f3 b8c6 d2d4 e5d4 f3d4 g8f6 b1c3 f8b4",
    "e2e4 e7e5 g1f3 b8c6 b1c3 g8f6 f1b5 f8b4 e1g1 e8g8",
    "e2e4 e7e5 g1f3 g8f6 f3e5 d7d6 e5f3 f6e4 d2d4 d6d5",
    "e2e4 e7e5 b1c3 g8f6 g1f3 b8c6 f1b5 f8b4 e1g1 e8g8",
    // Sicilian: open, dragon, Najdorf-ish.
    "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4 f3d4 g8f6 b1c3 a7a6",
    "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4 f3d4 g8f6 b1c3 g7g6",
    "e2e4 c7c5 g1f3 b8c6 d2d4 c5d4 f3d4 g8f6 b1c3 e7e5",
    "e2e4 c7c5 b1c3 b8c6 g1f3 e7e5 f1c4 f8e7 d2d3 d7d6",
    // French, Caro-Kann, Scandinavian, Pirc.
    "e2e4 e7e6 d2d4 d7d5 b1c3 g8f6 e4e5 f6d7 f2f4 c7c5",
    "e2e4 e7e6 d2d4 d7d5 e4e5 c7c5 c2c3 b8c6 g1f3 d8b6",
    "e2e4 c7c6 d2d4 d7d5 b1c3 d5e4 c3e4 c8f5 e4g3 f5g6",
    "e2e4 c7c6 d2d4 d7d5 e4e5 c8f5 c1e3 e7e6 c2c3 c6c5",
    "e2e4 d7d5 e4d5 d8d5 b1c3 d5a5 d2d4 g8f6 g1f3 c7c6",
    "e2e4 d7d6 d2d4 g8f6 b1c3 g7g6 g1f3 f8g7 f1e2 e8g8",
    // Queen's pawn: Queen's Gambit, Slav, Nimzo, King's Indian, London.
    "d2d4 d7d5 c2c4 e7e6 b1c3 g8f6 g1f3 f8e7 c1f4 e8g8",
    "d2d4 d7d5 c2c4 c7c6 g1f3 g8f6 b1c3 e7e6 e2e3 b8d7",
    "d2d4 g8f6 c2c4 e7e6 b1c3 f8b4 e2e3 e8g8 f1d3 d7d5",
    "d2d4 g8f6 c2c4 g7g6 b1c3 f8g7 e2e4 d7d6 g1f3 e8g8",
    "d2d4 g8f6 g1f3 g7g6 c1f4 f8g7 e2e3 e8g8 f1e2 d7d6",
    // Flank: English, Reti.
    "c2c4 e7e5 b1c3 g8f6 g1f3 b8c6 g2g3 f8b4 f1g2 e8g8",
];

/// A square in coordinate notation to a mailbox index.
fn parse_square(text: &[u8]) -> Option<u8> {
    let file = text.first()?.checked_sub(b'a')?;
    let rank = text.get(1)?.checked_sub(b'1')?;
    if file > 7 || rank > 7 {
        return None;
    }
    Some((START + (7 - rank as usize) * 10 + file as usize) as u8)
}

fn parse_move(text: &str) -> Option<(u8, u8)> {
    let bytes = text.as_bytes();
    if bytes.len() < 4 {
        return None;
    }
    Some((parse_square(&bytes[0..2])?, parse_square(&bytes[2..4])?))
}
