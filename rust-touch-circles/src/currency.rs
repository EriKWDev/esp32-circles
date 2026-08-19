//! What a krona is worth, from open.er-api.com - one of the few rate feeds still
//! served over plain HTTP and without a key.
//!
//! The feed quotes units per SEK, which is the awkward direction for everything
//! except the yen: 0.104 dollars per krona reads worse than 9.56 kronor per
//! dollar. So every row is inverted, and a currency worth less than a krona is
//! quoted per hundred of them instead of showing 0.06.

use core::fmt::Write as _;

use crate::fetch::{json_num, micros, scope};
use crate::font::FontId;
use crate::gfx::{Align, Scene, TextBuf, W, rgb};
use crate::net::{Buf, Net};

pub const HOST: &str = "open.er-api.com";
const PATH: &str = "/v6/latest/SEK";

/// The four asked for, plus one slot the keyboard fills in.
const FIXED: [&str; 4] = ["USD", "EUR", "JPY", "GBP"];
pub const ROWS: usize = FIXED.len() + 1;
pub const CUSTOM: usize = FIXED.len();

const ROW_H: i32 = 58;
const ROW_TOP: i32 = 104;
const INK: u16 = rgb(238, 245, 250);
const DIM: u16 = rgb(140, 152, 166);
const ACCENT: u16 = rgb(120, 220, 170);

pub const SYMBOL: (i32, i32, i32, i32) = (110, 408, 370, 460);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Cold,
    Fetching,
    Ready,
    Failed,
}

pub struct Currency {
    stage: Stage,
    /// Hundredths of a krona per unit, and how many units that price is for -
    /// one, or a hundred for the small currencies.
    price: [i32; ROWS],
    per: [i32; ROWS],
    custom: Buf<8>,
    /// What the feed resolved to last, for the About page.
    host_ip: Option<[u8; 4]>,
    /// Controller clock when the rates landed. Rates move slowly and a stale
    /// table looks exactly like a fresh one, so the page says which it is.
    refreshed: Option<(u8, u8)>,
}

impl Currency {
    pub const fn new() -> Self {
        Self {
            stage: Stage::Cold,
            price: [0; ROWS],
            per: [1; ROWS],
            custom: Buf::new(),
            host_ip: None,
            refreshed: None,
        }
    }

    pub fn host_ip(&self) -> Option<[u8; 4]> {
        self.host_ip
    }

    pub fn wake(&mut self) {
        if self.stage == Stage::Failed {
            self.stage = Stage::Cold;
        }
    }

    pub fn fingerprint(&self) -> u32 {
        self.stage as u32
            + self.price[0] as u32 * 8
            + self.custom.len() as u32 * 65_536
            + self.refreshed.map_or(0, |(hh, mm)| hh as u32 * 60 + mm as u32) * 1_048_576
    }

    pub fn custom_code(&self) -> &str {
        self.custom.as_str()
    }

    /// A new symbol means another fetch: the reply is far larger than there is
    /// room to keep, so the rates we want are pulled out of it as it arrives and
    /// the rest is gone.
    pub fn set_custom(&mut self, code: &str) {
        self.custom = Buf::new();
        for ch in code.chars().take(4) {
            let _ = write!(self.custom, "{}", ch.to_ascii_uppercase());
        }
        self.price[CUSTOM] = 0;
        self.stage = Stage::Cold;
    }

    pub fn step(&mut self, net: &mut Net, now_ms: u32, clock: Option<(u8, u8)>) {
        match self.stage {
            Stage::Cold => {
                if net.fetch.busy() {
                    return;
                }
                net.fetch_get(HOST, PATH, now_ms);
                self.stage = Stage::Fetching;
            }
            Stage::Fetching => {
                let resolved = net.fetch.resolved().map(|ip| ip.octets());
                if let Some(text) = net.fetch.take() {
                    self.host_ip = resolved;
                    self.refreshed = clock;
                    // Within `rates`, so a code can never be read off one of the
                    // metadata fields above it.
                    let rates = scope(text, "rates");
                    for index in 0..ROWS {
                        let code = if index == CUSTOM {
                            self.custom.as_str()
                        } else {
                            FIXED[index]
                        };
                        let (price, per) = if code.is_empty() {
                            (0, 1)
                        } else {
                            quote(rates, code)
                        };
                        self.price[index] = price;
                        self.per[index] = per;
                    }
                    self.stage = if self.price[0] > 0 {
                        Stage::Ready
                    } else {
                        Stage::Failed
                    };
                } else if net.fetch.take_failure() {
                    self.stage = Stage::Failed;
                }
            }
            Stage::Ready | Stage::Failed => {}
        }
    }

    pub fn draw(&self, scene: &mut Scene, alpha: u8) {
        scene.label(
            W as i32 / 2,
            54,
            FontId::Caption,
            DIM,
            alpha,
            Align::Center,
            match self.stage {
                Stage::Ready => "IN KRONOR",
                Stage::Failed => "NO RATES",
                _ => "RATES\u{2026}",
            },
        );

        for index in 0..ROWS {
            let y0 = ROW_TOP + index as i32 * ROW_H;
            let y1 = y0 + ROW_H - 8;
            let custom = index == CUSTOM;
            scene.pill(
                24,
                y0,
                W as i32 - 24,
                y1,
                14,
                if custom { rgb(28, 34, 42) } else { rgb(20, 24, 30) },
                alpha,
            );
            let code = if custom {
                self.custom.as_str()
            } else {
                FIXED[index]
            };
            let mut left = TextBuf::new();
            if code.is_empty() {
                let _ = write!(left, "ANY");
            } else if self.per[index] > 1 {
                let _ = write!(left, "{} {code}", self.per[index]);
            } else {
                let _ = write!(left, "{code}");
            }
            scene.label(
                46,
                (y0 + y1) / 2 + 13,
                FontId::Body,
                if custom { ACCENT } else { INK },
                alpha,
                Align::Left,
                left.as_str(),
            );

            let mut right = TextBuf::new();
            if self.price[index] > 0 {
                let _ = write!(
                    right,
                    "{}.{:02} KR",
                    self.price[index] / 100,
                    self.price[index] % 100
                );
            } else if code.is_empty() {
                let _ = write!(right, "TAP BELOW");
            } else {
                let _ = write!(right, "\u{2013}");
            }
            scene.label(
                W as i32 - 46,
                (y0 + y1) / 2 + 13,
                FontId::Body,
                if self.price[index] > 0 { INK } else { DIM },
                alpha,
                Align::Right,
                right.as_str(),
            );
        }

        // Where the numbers came from and when, in small print: a rate with no
        // provenance is just a number.
        let mut source = TextBuf::new();
        match self.refreshed {
            Some((hh, mm)) => {
                let _ = write!(source, "{HOST} {hh:02}:{mm:02}");
            }
            None => {
                let _ = write!(source, "{HOST}");
            }
        }
        scene.label(
            W as i32 / 2,
            398,
            FontId::Micro,
            DIM,
            alpha,
            Align::Center,
            source.as_str(),
        );

        let (x0, y0, x1, y1) = SYMBOL;
        scene.pill(x0, y0, x1, y1, (y1 - y0) / 2, rgb(26, 30, 38), alpha);
        scene.label(
            (x0 + x1) / 2,
            (y0 + y1) / 2 + 9,
            FontId::Caption,
            DIM,
            alpha,
            Align::Center,
            "SYMBOL",
        );
    }
}

/// Kronor per unit of `code`, in hundredths, and how many units that is for.
///
/// The feed gives units per krona, so this inverts. A currency smaller than a
/// krona is quoted per hundred units, because two decimals of 0.06 is not a
/// price anyone can use.
fn quote(text: &str, code: &str) -> (i32, i32) {
    let Some(rate) = json_num(text, code).and_then(micros) else {
        return (0, 1);
    };
    if rate <= 0 {
        return (0, 1);
    }
    // Both divisions are taken at the scale they will be shown at. Working out
    // the per-unit price first and multiplying by a hundred throws the answer
    // away: a yen comes out as 5 hundredths, and 5.00 kr per hundred is wrong by
    // a krona.
    let single = 100 * 1_000_000 / rate;
    if single >= 100 {
        (single as i32, 1)
    } else {
        ((10_000 * 1_000_000 / rate) as i32, 100)
    }
}
