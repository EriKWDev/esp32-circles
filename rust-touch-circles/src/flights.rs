//! The two aircraft nearest the panel.
//!
//! The controller does the work: both sources are HTTPS-only, and the
//! neighbourhood reply is thirteen kilobytes of which two aeroplanes are wanted,
//! so it filters as well as fetches. This reads the same kind of line records the
//! dump and the quotes use.
//!
//! Aircraft move, so this refreshes every ten seconds - but only while the page is
//! open. Nothing here polls in the background.

use core::fmt::Write as _;

use crate::font::FontId;
use crate::gfx::{Align, Scene, TextBuf, W, muted, rgb};
use crate::store::FixedStr;

pub const MAX_FLIGHTS: usize = 2;
/// Half a minute. Ten seconds was tried and is more often than the picture
/// changes usefully, given that only aircraft inside twenty kilometres are
/// listed at all.
const REFRESH_MS: u32 = 30_000;

const INK: u16 = rgb(238, 245, 250);
const DIM: u16 = rgb(140, 152, 166);
const PLATE: u16 = rgb(18, 22, 30);
pub const ACCENT: u16 = rgb(120, 200, 255);

/// The heading band, which is also the button that changes where "overhead" is.
pub const HEADER: (i32, i32, i32, i32) = (0, 30, 480, 100);

/// Places to look from, beyond wherever the panel thinks it is. Coordinates as
/// text because that is the form they go back out in - the endpoint takes them
/// straight into a URL.
/// Bjärred first, because it is where the panel lives - the geo-IP guess is the
/// fallback rather than the default now.
pub const PLACES: [(&str, &str, &str); 4] = [
    ("BJÄRRED", "55.7183", "13.0264"),
    ("LUND", "55.7047", "13.1910"),
    ("LANDVETTER", "57.6628", "12.2798"),
    ("HERE", "", ""),
];

const CARD_TOP: i32 = 108;
const CARD_H: i32 = 168;
const CARD_GAP: i32 = 14;

#[derive(Clone, Copy)]
struct Flight {
    callsign: FixedStr<10>,
    from: FixedStr<5>,
    to: FixedStr<5>,
    kind: FixedStr<6>,
    /// Which way to look: eight-point, from the place being looked from.
    bearing: FixedStr<3>,
    /// The towns, as the controller spells them out.
    from_town: FixedStr<18>,
    to_town: FixedStr<18>,
    knots: u32,
    feet: u32,
    /// Tenths of a kilometre, as the controller sends it.
    dkm: u32,
}

const NO_FLIGHT: Flight = Flight {
    callsign: FixedStr::EMPTY,
    from: FixedStr::EMPTY,
    to: FixedStr::EMPTY,
    kind: FixedStr::EMPTY,
    bearing: FixedStr::EMPTY,
    from_town: FixedStr::EMPTY,
    to_town: FixedStr::EMPTY,
    knots: 0,
    feet: 0,
    dkm: 0,
};

pub struct Flights {
    seen: [Flight; MAX_FLIGHTS],
    n: usize,
    asking: bool,
    /// Set until the first answer, so an empty screen says why.
    waiting: bool,
    failed: bool,
    next_ask_ms: u32,
    /// Which of PLACES to look from. Zero is wherever the panel is.
    pub place: usize,
}

impl Flights {
    pub const fn new() -> Self {
        Self {
            seen: [NO_FLIGHT; MAX_FLIGHTS],
            n: 0,
            asking: false,
            waiting: true,
            failed: false,
            next_ask_ms: 0,
            place: 0,
        }
    }

    /// Opening the page. What was on screen stays there until the first answer,
    /// which is a second or so away.
    pub fn open(&mut self, now_ms: u32) {
        self.asking = false;
        self.waiting = self.n == 0;
        self.failed = false;
        self.next_ask_ms = now_ms;
    }

    /// Next place in the list, and ask again at once - the answer for one place
    /// says nothing about another.
    pub fn cycle_place(&mut self, now_ms: u32) {
        self.place = (self.place + 1) % PLACES.len();
        self.n = 0;
        self.asking = false;
        self.waiting = true;
        self.next_ask_ms = now_ms;
    }

    /// Where to look from: the chosen place, or nothing when it is "here" and the
    /// caller should use the panel's own position.
    pub fn chosen(&self) -> (&'static str, &'static str) {
        let (_, lat, lon) = PLACES[self.place.min(PLACES.len() - 1)];
        (lat, lon)
    }

    pub fn place_name(&self) -> &'static str {
        PLACES[self.place.min(PLACES.len() - 1)].0
    }

    pub fn due(&self, now_ms: u32) -> bool {
        !self.asking && now_ms.wrapping_sub(self.next_ask_ms) < u32::MAX / 2
    }

    pub fn asked(&mut self, now_ms: u32) {
        self.asking = true;
        self.next_ask_ms = now_ms + REFRESH_MS;
    }

    pub fn absorb(&mut self, text: &str) {
        self.asking = false;
        self.waiting = false;
        self.failed = false;
        self.n = 0;
        for line in text.lines() {
            // Town names arrive as their own records, since a line can carry only
            // one field of free text and this needs two.
            if let Some(rest) = line.strip_prefix("n:") {
                let mut fields = rest.split(':');
                let (Some(index), Some(end), Some(town)) =
                    (fields.next(), fields.next(), fields.next())
                else {
                    continue;
                };
                let Ok(index) = index.parse::<usize>() else {
                    continue;
                };
                if index < self.n {
                    if end == "0" {
                        self.seen[index].from_town = FixedStr::new(town);
                    } else {
                        self.seen[index].to_town = FixedStr::new(town);
                    }
                }
                continue;
            }
            let Some(rest) = line.strip_prefix("f:") else {
                continue;
            };
            if self.n >= MAX_FLIGHTS {
                break;
            }
            // callsign:from:to:knots:feet:km:type - the type is last and may be
            // absent, which is why it is read with a default rather than required.
            let mut fields = rest.split(':');
            let (Some(callsign), Some(from), Some(to), Some(knots), Some(feet), Some(km)) = (
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
            ) else {
                continue;
            };
            // Bearing sits between the distance and the type: eight-point, from
            // the place being looked from.
            let bearing = fields.next().unwrap_or("");
            let (whole, tenth) = km.split_once('.').unwrap_or((km, "0"));
            self.seen[self.n] = Flight {
                callsign: FixedStr::new(callsign),
                from: FixedStr::new(from),
                to: FixedStr::new(to),
                bearing: FixedStr::new(bearing),
                kind: FixedStr::new(fields.next().unwrap_or("")),
                from_town: FixedStr::EMPTY,
                to_town: FixedStr::EMPTY,
                knots: knots.parse().unwrap_or(0),
                feet: feet.parse().unwrap_or(0),
                dkm: whole.parse::<u32>().unwrap_or(0) * 10
                    + tenth.as_bytes().first().map_or(0, |b| (b - b'0') as u32),
            };
            self.n += 1;
        }
        if self.n == 0 {
            // Nothing overhead is an answer, not a failure.
            self.failed = false;
        }
    }

    pub fn failed(&mut self) {
        self.asking = false;
        self.waiting = false;
        self.failed = true;
    }

    /// Cheap summary, so the loop can tell when a refresh changed anything.
    #[allow(dead_code)]
    pub fn fingerprint(&self) -> u32 {
        let mut sum = self.n as u32 + self.failed as u32 * 2 + self.waiting as u32 * 4;
        for flight in self.seen[..self.n].iter() {
            sum = sum.wrapping_mul(31).wrapping_add(flight.feet + flight.dkm + flight.knots);
        }
        sum
    }

    pub fn draw(&self, scene: &mut Scene, city: &str, clock: Option<(u8, u8)>, alpha: u8) {
        scene.label(
            42,
            62,
            FontId::Body,
            INK,
            alpha,
            Align::Left,
            "FLIGHTS",
        );
        if let Some((hh, mm)) = clock {
            let mut time = TextBuf::new();
            let _ = write!(time, "{hh:02}:{mm:02}");
            scene.label(
                W as i32 - 42,
                62,
                FontId::Body,
                DIM,
                alpha,
                Align::Right,
                time.as_str(),
            );
        }
        // Where "overhead" is - a geo-IP guess unless a place was chosen, so it
        // says which, and the heading is the button that changes it.
        let mut where_ = TextBuf::new();
        if self.chosen().0.is_empty() {
            let _ = write!(where_, "{}", if city.is_empty() { "HERE" } else { city });
        } else {
            let _ = write!(where_, "{}", self.place_name());
        }
        scene.label(
            42,
            86,
            FontId::Micro,
            muted(ACCENT),
            alpha,
            Align::Left,
            where_.as_str(),
        );

        if self.n == 0 {
            let message = if self.failed {
                "NO ANSWER"
            } else if self.waiting {
                "LOOKING UP\u{2026}"
            } else {
                "NOTHING NEARBY"
            };
            scene.label(
                W as i32 / 2,
                250,
                FontId::Body,
                DIM,
                alpha,
                Align::Center,
                message,
            );
            return;
        }

        for (index, flight) in self.seen[..self.n].iter().enumerate() {
            let y0 = CARD_TOP + index as i32 * (CARD_H + CARD_GAP);
            let y1 = y0 + CARD_H;
            scene.pill(20, y0, W as i32 - 20, y1, 16, PLATE, alpha);

            scene.label(
                42,
                y0 + 46,
                FontId::Body,
                ACCENT,
                alpha,
                Align::Left,
                flight.callsign.as_str(),
            );
            if !flight.kind.is_empty() {
                scene.label(
                    W as i32 - 42,
                    y0 + 42,
                    FontId::Micro,
                    DIM,
                    alpha,
                    Align::Right,
                    flight.kind.as_str(),
                );
            }

            // The route, with a chevron for the arrow the font does not have.
            let mut route = TextBuf::new();
            let _ = write!(route, "{} > {}", flight.from.as_str(), flight.to.as_str());
            scene.label(
                42,
                y0 + 86,
                FontId::Caption,
                INK,
                alpha,
                Align::Left,
                route.as_str(),
            );
            // The same route in words, small, under the codes - three letters are
            // no help unless you already know them.
            if !flight.from_town.is_empty() || !flight.to_town.is_empty() {
                let mut towns = TextBuf::new();
                let _ = write!(
                    towns,
                    "{} > {}",
                    flight.from_town.as_str(),
                    flight.to_town.as_str()
                );
                scene.label(
                    42,
                    y0 + 112,
                    FontId::Micro,
                    muted(INK),
                    alpha,
                    Align::Left,
                    towns.as_str(),
                );
            }

            let mut distance = TextBuf::new();
            let _ = write!(
                distance,
                "{} {}.{} KM",
                flight.bearing.as_str(),
                flight.dkm / 10,
                flight.dkm % 10
            );
            scene.label(
                W as i32 - 42,
                y0 + 86,
                FontId::Caption,
                muted(ACCENT),
                alpha,
                Align::Right,
                distance.as_str(),
            );

            let mut numbers = TextBuf::new();
            let _ = write!(numbers, "{} KT  {} FT", flight.knots, flight.feet);
            scene.label(
                42,
                y0 + 142,
                FontId::Micro,
                DIM,
                alpha,
                Align::Left,
                numbers.as_str(),
            );
        }
    }
}
