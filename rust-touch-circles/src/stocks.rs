//! Share prices for three watchlists.
//!
//! The quotes come from the controller, not from here: the panel has no TLS and
//! every feed worth using is HTTPS-only, so the irrigation controller - which
//! already links libcurl and is already this panel's gateway - fetches them and
//! answers in the same line-oriented format its other endpoints use.
//!
//! Six symbols per request is the endpoint's limit, one upstream call each, so a
//! longer list arrives a page at a time and fills in as it comes.

use core::fmt::Write as _;

use crate::font::FontId;
use crate::gfx::{Align, Scene, TextBuf, W, muted, rgb};
use crate::store::{MAX_LISTS, MAX_SYMBOLS, Settings};

/// What one request may ask for, matching the endpoint's own cap.
pub const PER_REQUEST: usize = 6;
/// A quote older than this is refetched when the page is opened again.
const STALE_MS: u32 = 120_000;

const INK: u16 = rgb(238, 245, 250);
const DIM: u16 = rgb(140, 152, 166);
const UP: u16 = rgb(90, 220, 130);
const DOWN: u16 = rgb(245, 100, 95);
const PLATE: u16 = rgb(20, 24, 30);
pub const ACCENT: u16 = rgb(250, 210, 120);

/// Row geometry. Three rows fit under the tabs with room for the name and the
/// small print beneath each price.
pub const ROW_TOP: i32 = 128;
pub const ROW_H: i32 = 104;
pub const ROWS_SHOWN: usize = 3;
pub const TAB_Y: (i32, i32) = (58, 104);
/// The scroll band: everything below the tabs belongs to the list.
pub const LIST_TOP: i32 = 120;

#[derive(Clone, Copy)]
pub struct Quote {
    /// Hundredths of the quote currency, and the previous close in the same.
    pub price: i32,
    pub prev: i32,
    /// Seconds since the epoch, as the feed reported it.
    pub when: u32,
    pub currency: crate::store::FixedStr<4>,
    pub name: crate::store::FixedStr<28>,
    pub known: bool,
    pub bad: bool,
}

const NO_QUOTE: Quote = Quote {
    price: 0,
    prev: 0,
    when: 0,
    currency: crate::store::FixedStr::EMPTY,
    name: crate::store::FixedStr::EMPTY,
    known: false,
    bad: false,
};

pub struct Stocks {
    /// Which list is on screen.
    pub list: usize,
    /// One quote per symbol of the current list, in the list's order.
    quotes: [Quote; MAX_SYMBOLS],
    /// Scroll offset in pixels, and where the current drag started.
    pub scroll: i32,
    drag_from: Option<(i32, i32)>,
    drag_scroll: i32,
    /// How far through the list the fetching has got, and when it began.
    fetched: usize,
    fetching: bool,
    last_fetch_ms: u32,
    pub waiting: bool,
}

impl Stocks {
    pub const fn new() -> Self {
        Self {
            list: 0,
            quotes: [NO_QUOTE; MAX_SYMBOLS],
            scroll: 0,
            drag_from: None,
            drag_scroll: 0,
            fetched: 0,
            fetching: false,
            last_fetch_ms: 0,
            waiting: false,
        }
    }

    /// Opening the page, or switching tabs: start again from the top of the list.
    pub fn open(&mut self, list: usize, now_ms: u32) {
        if list != self.list {
            self.scroll = 0;
        }
        self.list = list.min(MAX_LISTS - 1);
        self.quotes = [NO_QUOTE; MAX_SYMBOLS];
        self.fetched = 0;
        self.fetching = false;
        self.waiting = true;
        // Backdated so the first pass asks immediately.
        self.last_fetch_ms = now_ms.wrapping_sub(STALE_MS);
    }

    /// The symbols still to be asked for, as the endpoint wants them: comma
    /// separated, at most a page at a time.
    pub fn next_page(&mut self, settings: &Settings) -> Option<crate::net::Buf<80>> {
        let list = settings.lists.get(self.list)?;
        if self.fetched >= list.n_symbols {
            return None;
        }
        let mut out = crate::net::Buf::<80>::new();
        let end = (self.fetched + PER_REQUEST).min(list.n_symbols);
        for index in self.fetched..end {
            if index > self.fetched {
                let _ = write!(out, ",");
            }
            let _ = write!(out, "{}", list.symbols[index].as_str());
        }
        Some(out)
    }

    /// Called once a page has been asked for, so the next one follows it.
    pub fn page_sent(&mut self, settings: &Settings, now_ms: u32) {
        let count = settings
            .lists
            .get(self.list)
            .map_or(0, |list| list.n_symbols);
        self.fetched = (self.fetched + PER_REQUEST).min(count);
        self.fetching = true;
        self.last_fetch_ms = now_ms;
    }

    pub fn due(&self, settings: &Settings, now_ms: u32) -> bool {
        let count = settings
            .lists
            .get(self.list)
            .map_or(0, |list| list.n_symbols);
        if self.fetching {
            return false;
        }
        self.fetched < count || now_ms.wrapping_sub(self.last_fetch_ms) > STALE_MS
    }

    /// A page has arrived; the whole list is refetched when it goes stale.
    pub fn absorb(&mut self, text: &str, settings: &Settings, now_ms: u32) {
        self.fetching = false;
        self.waiting = false;
        self.last_fetch_ms = now_ms;
        let Some(list) = settings.lists.get(self.list) else {
            return;
        };
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("q:") {
                let mut fields = rest.split(':');
                let (Some(symbol), Some(price), Some(prev), Some(when), Some(currency)) = (
                    fields.next(),
                    fields.next(),
                    fields.next(),
                    fields.next(),
                    fields.next(),
                ) else {
                    continue;
                };
                // The name is the rest of the line: it may contain a colon, which
                // is why the record puts it last.
                let name = fields.next().unwrap_or("");
                let Some(slot) = list.symbols[..list.n_symbols]
                    .iter()
                    .position(|s| s.as_str() == symbol)
                else {
                    continue;
                };
                self.quotes[slot] = Quote {
                    price: hundredths(price),
                    prev: hundredths(prev),
                    when: when.parse().unwrap_or(0),
                    currency: crate::store::FixedStr::new(currency),
                    name: crate::store::FixedStr::new(name),
                    known: true,
                    bad: false,
                };
            } else if let Some(symbol) = line.strip_prefix("bad:")
                && let Some(slot) = list.symbols[..list.n_symbols]
                    .iter()
                    .position(|s| s.as_str() == symbol)
            {
                self.quotes[slot] = Quote {
                    bad: true,
                    ..NO_QUOTE
                };
            }
        }
        // A stale sweep starts over from the top.
        if self.fetched >= list.n_symbols && now_ms.wrapping_sub(self.last_fetch_ms) > STALE_MS {
            self.fetched = 0;
        }
    }

    pub fn failed(&mut self) {
        self.fetching = false;
        self.waiting = false;
    }

    pub fn press(&mut self, x: i32, y: i32) {
        self.drag_from = Some((x, y));
        self.drag_scroll = self.scroll;
    }

    /// Dragging scrolls the list. Rows are too tall for a scrollbar to be worth
    /// the width, so the list itself is the control.
    pub fn drag(&mut self, y: i32, settings: &Settings) {
        let Some((_, from_y)) = self.drag_from else {
            return;
        };
        let count = settings
            .lists
            .get(self.list)
            .map_or(0, |list| list.n_symbols) as i32;
        let span = (count - ROWS_SHOWN as i32).max(0) * ROW_H;
        self.scroll = (self.drag_scroll - (y - from_y)).clamp(0, span);
    }

    pub fn release(&mut self) {
        self.drag_from = None;
    }

    /// Which row a touch landed on, for the editor.
    pub fn row_at(&self, y: i32, settings: &Settings) -> Option<usize> {
        if y < LIST_TOP {
            return None;
        }
        let count = settings
            .lists
            .get(self.list)
            .map_or(0, |list| list.n_symbols);
        let index = ((y - ROW_TOP + self.scroll) / ROW_H) as usize;
        (index < count).then_some(index)
    }

    pub fn draw(&self, scene: &mut Scene, settings: &Settings, alpha: u8) {
        // Tabs: three names across the top, the current one filled.
        let width = (W as i32 - 32) / MAX_LISTS as i32;
        for index in 0..MAX_LISTS {
            let x0 = 16 + index as i32 * width;
            let here = index == self.list;
            scene.pill(
                x0 + 3,
                TAB_Y.0,
                x0 + width - 3,
                TAB_Y.1,
                12,
                if here { ACCENT } else { PLATE },
                alpha,
            );
            let name = settings.lists[index].name.as_str();
            scene.label(
                x0 + width / 2,
                TAB_Y.0 + 31,
                FontId::Micro,
                if here { rgb(10, 12, 16) } else { DIM },
                alpha,
                Align::Center,
                if name.is_empty() { "LIST" } else { name },
            );
        }

        let Some(list) = settings.lists.get(self.list) else {
            return;
        };
        if list.n_symbols == 0 {
            scene.label(
                W as i32 / 2,
                250,
                FontId::Body,
                DIM,
                alpha,
                Align::Center,
                "NO SYMBOLS",
            );
            return;
        }

        for index in 0..list.n_symbols {
            let y = ROW_TOP + index as i32 * ROW_H - self.scroll;
            // Only what is on screen: rows scrolled past the edges would spend
            // primitives on nothing.
            if y + ROW_H < LIST_TOP || y > 480 {
                continue;
            }
            let quote = self.quotes[index];
            let symbol = list.symbols[index].as_str();

            scene.label(
                26,
                y + 30,
                FontId::Body,
                INK,
                alpha,
                Align::Left,
                symbol,
            );

            if quote.bad {
                scene.label(
                    W as i32 - 26,
                    y + 30,
                    FontId::Caption,
                    DOWN,
                    alpha,
                    Align::Right,
                    "NO SUCH SYMBOL",
                );
                continue;
            }
            if !quote.known {
                scene.label(
                    W as i32 - 26,
                    y + 30,
                    FontId::Caption,
                    DIM,
                    alpha,
                    Align::Right,
                    "\u{2026}",
                );
                continue;
            }

            // Green up, red down, against the previous close - which is what the
            // feed gives and what a day's move means.
            let move_ = quote.price - quote.prev;
            let tint = if move_ >= 0 { UP } else { DOWN };
            let mut price = TextBuf::new();
            let _ = write!(price, "{}.{:02}", quote.price / 100, quote.price % 100);
            scene.label(
                W as i32 - 26,
                y + 34,
                FontId::Body,
                tint,
                alpha,
                Align::Right,
                price.as_str(),
            );

            let mut change = TextBuf::new();
            let permille = if quote.prev > 0 {
                (move_ as i64 * 1000 / quote.prev as i64) as i32
            } else {
                0
            };
            let _ = write!(
                change,
                "{}{}.{}%",
                if move_ >= 0 { "+" } else { "-" },
                permille.abs() / 10,
                permille.abs() % 10
            );
            scene.label(
                W as i32 - 26,
                y + 62,
                FontId::Micro,
                tint,
                alpha,
                Align::Right,
                change.as_str(),
            );

            // Full name and the quote's own date, in small print.
            scene.label(
                26,
                y + 56,
                FontId::Micro,
                DIM,
                alpha,
                Align::Left,
                quote.name.as_str(),
            );
            let mut stamp = TextBuf::new();
            let (year, month, day, hour, minute) = civil(quote.when);
            let _ = write!(
                stamp,
                "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} {}",
                quote.currency.as_str()
            );
            scene.label(
                26,
                y + 80,
                FontId::Micro,
                muted(DIM),
                alpha,
                Align::Left,
                stamp.as_str(),
            );
        }
    }
}

/// A decimal price as hundredths. The feed sends four decimal places; two is what
/// a price is read in and the rest is noise at this size.
fn hundredths(text: &str) -> i32 {
    let (whole, frac) = text.split_once('.').unwrap_or((text, "0"));
    let whole: i32 = whole.parse().unwrap_or(0);
    let mut cents = 0;
    let mut scale = 10;
    for byte in frac.bytes().take(2) {
        if !byte.is_ascii_digit() {
            break;
        }
        cents += (byte - b'0') as i32 * scale;
        scale /= 10;
    }
    whole * 100 + cents
}

/// Epoch seconds to a civil date, by the same algorithm the build stamp uses. UTC:
/// the panel has no timezone of its own, and a quote's date matters more than its
/// minute.
fn civil(epoch: u32) -> (i32, u32, u32, u32, u32) {
    let days = (epoch / 86_400) as i64;
    let rest = epoch % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        year as i32,
        month as u32,
        day as u32,
        rest / 3600,
        rest / 60 % 60,
    )
}
