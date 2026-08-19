//! A calculator: the four operations, a square root, pi, and one memory.
//!
//! Arithmetic is f32 through the soft-float routines - there is no FPU here, but a
//! calculator does a handful of operations per keypress, not per pixel, so the
//! cost is irrelevant and fixed point would only add its own rounding to explain.
//!
//! Entry is kept as text until it is needed as a number, which is what lets
//! "0.000" and "12." exist while they are being typed.

use crate::font::FontId;
use crate::gfx::{Align, H, Scene, TextBuf, W, muted, rgb};

const COLS: usize = 5;
const ROWS: usize = 5;
const PAD: i32 = 6;
const TOP: i32 = 150;

/// Row-major. The last two are one wide equals key.
const KEYS: [&str; COLS * ROWS] = [
    "MC", "MR", "M+", "C", "/", //
    "7", "8", "9", "sqrt", "*", //
    "4", "5", "6", "pi", "-", //
    "1", "2", "3", "+/-", "+", //
    "0", ".", "%", "=", "=",
];

const DIGIT: u16 = rgb(46, 52, 62);
const OP: u16 = rgb(232, 140, 60);
const FUNC: u16 = rgb(70, 82, 98);
const INK: u16 = rgb(240, 245, 250);

pub struct Calc {
    /// What is being typed, or the last result rendered back into text.
    entry: TextBuf,
    /// Left-hand side and the operation waiting for its right-hand side.
    left: f32,
    op: Option<char>,
    memory: f32,
    /// Set once a result is shown, so the next digit starts a fresh entry rather
    /// than appending to the answer.
    settled: bool,
    error: bool,
}

impl Calc {
    pub const fn new() -> Self {
        Self {
            entry: TextBuf::new(),
            left: 0.0,
            op: None,
            memory: 0.0,
            settled: true,
            error: false,
        }
    }

    pub fn restart(&mut self) {
        *self = Self::new();
    }

    fn value(&self) -> f32 {
        self.entry.as_str().parse::<f32>().unwrap_or(0.0)
    }

    fn show(&mut self, value: f32) {
        use core::fmt::Write as _;
        self.entry = TextBuf::new();
        // Whole numbers lose their point; everything else keeps enough digits to be
        // useful without turning into noise. `trunc` is a std intrinsic, so the
        // round trip through i64 is the no_std way of asking.
        if value.abs() < 1e9 && value == (value as i64) as f32 {
            let _ = write!(self.entry, "{}", value as i64);
        } else {
            let _ = write!(self.entry, "{value:.6}");
            // Trim the zeros that formatting always pads with.
            let text = self.entry.as_str();
            let trimmed = text.trim_end_matches('0').trim_end_matches('.');
            let keep = TextBuf::from_str(trimmed);
            self.entry = keep;
        }
        self.settled = true;
    }

    pub fn key_at(x: i32, y: i32) -> Option<usize> {
        (0..COLS * ROWS).find(|index| {
            let (x0, y0, x1, y1) = Self::key_rect(*index);
            x >= x0 && x <= x1 && y >= y0 && y <= y1
        })
    }

    pub fn key_rect(index: usize) -> (i32, i32, i32, i32) {
        let width = (W as i32 - PAD * (COLS as i32 + 1)) / COLS as i32;
        let height = (H as i32 - TOP - PAD * (ROWS as i32 + 1)) / ROWS as i32;
        let col = (index % COLS) as i32;
        let row = (index / COLS) as i32;
        let x0 = PAD + col * (width + PAD);
        let y0 = TOP + PAD + row * (height + PAD);
        // The equals key is the last two cells joined.
        let span = if index == COLS * ROWS - 2 { 2 } else { 1 };
        (x0, y0, x0 + width * span + PAD * (span - 1), y0 + height)
    }

    pub fn press(&mut self, index: usize) {
        let key = KEYS[index.min(KEYS.len() - 1)];
        if self.error && key != "C" {
            return;
        }
        match key {
            "C" => *self = Self { memory: self.memory, ..Self::new() },
            "MC" => self.memory = 0.0,
            "MR" => {
                let memory = self.memory;
                self.show(memory);
            }
            "M+" => self.memory += self.value(),
            "pi" => self.show(core::f32::consts::PI),
            "sqrt" => {
                let value = self.value();
                if value < 0.0 {
                    self.error = true;
                } else {
                    self.show(sqrt(value));
                }
            }
            "+/-" => {
                let value = -self.value();
                self.show(value);
            }
            "%" => {
                let value = self.value() / 100.0;
                self.show(value);
            }
            "+" | "-" | "*" | "/" => {
                // Chaining applies what is pending first, so 2+3+4 works without
                // pressing equals in between.
                if self.op.is_some() && !self.settled {
                    self.equals();
                } else {
                    self.left = self.value();
                }
                self.op = key.chars().next();
                self.settled = true;
            }
            "=" => self.equals(),
            digit => {
                if self.settled {
                    self.entry = TextBuf::new();
                    self.settled = false;
                }
                if digit == "." && self.entry.as_str().contains('.') {
                    return;
                }
                if self.entry.as_str().len() < 12 {
                    use core::fmt::Write as _;
                    let _ = write!(self.entry, "{digit}");
                }
            }
        }
    }

    fn equals(&mut self) {
        let right = self.value();
        let result = match self.op {
            Some('+') => self.left + right,
            Some('-') => self.left - right,
            Some('*') => self.left * right,
            Some('/') => {
                if right == 0.0 {
                    self.error = true;
                    return;
                }
                self.left / right
            }
            _ => right,
        };
        self.op = None;
        self.left = result;
        self.show(result);
    }

    pub fn draw(&self, scene: &mut Scene, alpha: u8) {
        // Readout.
        scene.pill(PAD, 96, W as i32 - PAD, TOP - 6, 12, rgb(20, 24, 30), alpha);
        let text = if self.error {
            "ERROR"
        } else if self.entry.as_str().is_empty() {
            "0"
        } else {
            self.entry.as_str()
        };
        scene.label(
            W as i32 - PAD - 14,
            TOP - 24,
            FontId::Display,
            INK,
            alpha,
            Align::Right,
            text,
        );

        // What is pending, and whether anything is in memory - small, on the left,
        // so the number itself stays the loudest thing on the screen.
        let mut status = TextBuf::new();
        use core::fmt::Write as _;
        if let Some(op) = self.op {
            let symbol = match op {
                '*' => "\u{d7}",
                '/' => "\u{f7}",
                '+' => "+",
                _ => "-",
            };
            let _ = write!(status, "{} {}", self.left as i64, symbol);
        }
        if self.memory != 0.0 {
            let _ = write!(status, "  M");
        }
        scene.label(
            PAD + 16,
            126,
            FontId::Caption,
            rgb(140, 150, 165),
            alpha,
            Align::Left,
            status.as_str(),
        );

        for index in 0..COLS * ROWS {
            // The cell swallowed by the wide equals key draws nothing.
            if index == COLS * ROWS - 1 {
                continue;
            }
            let (x0, y0, x1, y1) = Self::key_rect(index);
            let key = KEYS[index];
            let plate = match key {
                "+" | "-" | "*" | "/" | "=" => OP,
                "MC" | "MR" | "M+" | "C" | "sqrt" | "pi" | "+/-" | "%" => FUNC,
                _ => DIGIT,
            };
            scene.pill(x0, y0, x1, y1, 12, plate, alpha);
            // The symbols are real glyphs now - the font was baked without them, so
            // every one of these keys was previously blank.
            let caption = match key {
                "sqrt" => "\u{221a}",
                "pi" => "\u{3c0}",
                "*" => "\u{d7}",
                "/" => "\u{f7}",
                other => other,
            };
            // Anything longer than a single character goes down a size to fit, and
            // to a face that has letters: Micro carries digits only.
            let font = if caption.chars().count() > 1 {
                FontId::Caption
            } else {
                FontId::Body
            };
            let f = font.get();
            scene.label(
                (x0 + x1) / 2,
                (y0 + y1) / 2 + f.ascent / 2 - f.ascent / 8,
                font,
                if plate == DIGIT { INK } else { muted(INK) },
                alpha,
                Align::Center,
                caption,
            );
        }
    }
}

/// Newton's method, because there is no hardware square root and `f32::sqrt` is a
/// std intrinsic that no_std does not provide.
fn sqrt(value: f32) -> f32 {
    if value <= 0.0 {
        return 0.0;
    }
    let mut guess = value;
    let mut index = 0;
    while index < 20 {
        guess = 0.5 * (guess + value / guess);
        index += 1;
    }
    guess
}
