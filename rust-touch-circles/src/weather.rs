//! Tomorrow's weather, for wherever the panel happens to be.
//!
//! Two fetches: ip-api.com for the coordinates and the city name, then
//! open-meteo for the forecast at those coordinates. Open-meteo because it is
//! the one good source still served over plain HTTP - met.no redirects to https
//! and DMI wants an API key - and for Nordic coordinates it runs MET Norway's
//! own model, so the numbers are yr.no's numbers.
//!
//! The forecast opens on tomorrow rather than today, which is what the panel is
//! for: whether to let the schedule water in the morning.

use core::fmt::Write as _;

use crate::fetch::{json_array, json_num, json_str, tenths};
use crate::font::FontId;
use crate::gfx::{Align, Scene, TextBuf, W, muted, rgb};
use crate::net::{Buf, Net};

pub const MAX_DAYS: usize = 6;

const HOST_GEO: &str = "ip-api.com";
const PATH_GEO: &str = "/json/?fields=lat,lon,city";
const HOST_MET: &str = "api.open-meteo.com";

const INK: u16 = rgb(238, 245, 250);
const DIM: u16 = rgb(140, 152, 166);
const C_SUN: u16 = rgb(255, 206, 84);
const C_CLOUD: u16 = rgb(176, 190, 205);
const C_RAIN: u16 = rgb(96, 176, 255);
const C_SNOW: u16 = rgb(226, 240, 255);
const C_BOLT: u16 = rgb(255, 226, 120);

pub const PREV: (i32, i32, i32, i32) = (16, 404, 148, 460);
pub const NEXT: (i32, i32, i32, i32) = (332, 404, 464, 460);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Nothing asked for yet, or a retry is due.
    Cold,
    Locating,
    Forecasting,
    Ready,
    Failed,
}

#[derive(Clone, Copy)]
struct Day {
    /// Weekday, 0 = Monday, and the day of the month, both from the ISO date.
    weekday: u8,
    dom: u8,
    month: u8,
    code: u16,
    /// Tenths of a degree, and tenths of a millimetre.
    high: i32,
    low: i32,
    rain: i32,
}

const NO_DAY: Day = Day {
    weekday: 0,
    dom: 0,
    month: 0,
    code: 0,
    high: 0,
    low: 0,
    rain: 0,
};

pub struct Weather {
    stage: Stage,
    city: Buf<28>,
    lat: Buf<12>,
    lon: Buf<12>,
    days: [Day; MAX_DAYS],
    n: usize,
    /// Which day is on screen. Starts on tomorrow.
    day: usize,
}

impl Weather {
    pub const fn new() -> Self {
        Self {
            stage: Stage::Cold,
            city: Buf::new(),
            lat: Buf::new(),
            lon: Buf::new(),
            days: [NO_DAY; MAX_DAYS],
            n: 0,
            day: 1,
        }
    }

    /// Called when the page is opened. A forecast in hand is kept - it is hours
    /// fresh - but a failure is worth another try.
    pub fn wake(&mut self) {
        if self.stage == Stage::Failed {
            self.stage = Stage::Cold;
        }
        self.day = 1;
    }

    /// Cheap summary of what is on screen, so the loop can tell when a fetch has
    /// changed something worth repainting.
    pub fn fingerprint(&self) -> u32 {
        self.stage as u32
            + self.n as u32 * 8
            + self.day as u32 * 64
            + self.city.len() as u32 * 1024
    }

    pub fn next_day(&mut self) {
        if self.n > 0 {
            self.day = (self.day + 1) % self.n;
        }
    }

    pub fn prev_day(&mut self) {
        if self.n > 0 {
            self.day = (self.day + self.n - 1) % self.n;
        }
    }

    /// Advance the two-fetch sequence. Does nothing once a forecast is in.
    ///
    /// The replies are read inside the borrow that produced them and copied into
    /// our own fields, because issuing the next request needs the network back.
    pub fn step(&mut self, net: &mut Net, now_ms: u32) {
        match self.stage {
            Stage::Cold => {
                if net.fetch.busy() {
                    return;
                }
                net.fetch_get(HOST_GEO, PATH_GEO, now_ms);
                self.stage = Stage::Locating;
            }
            Stage::Locating => {
                let mut located = false;
                if let Some(text) = net.fetch.take() {
                    self.city = Buf::new();
                    self.lat = Buf::new();
                    self.lon = Buf::new();
                    if let Some(city) = json_str(text, "city") {
                        let _ = write!(self.city, "{city}");
                    }
                    // Kept as text: they only ever go back out in a URL, and
                    // parsing them would mean formatting a float to rebuild it.
                    if let (Some(lat), Some(lon)) = (json_num(text, "lat"), json_num(text, "lon")) {
                        let _ = write!(self.lat, "{lat}");
                        let _ = write!(self.lon, "{lon}");
                        located = true;
                    }
                } else if net.fetch.take_failure() {
                    self.stage = Stage::Failed;
                    return;
                }
                if located {
                    let mut path = Buf::<200>::new();
                    let _ = write!(
                        path,
                        "/v1/forecast?latitude={}&longitude={}&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_sum&timezone=auto&forecast_days={MAX_DAYS}",
                        self.lat.as_str(),
                        self.lon.as_str()
                    );
                    net.fetch_get(HOST_MET, path.as_str(), now_ms);
                    self.stage = Stage::Forecasting;
                } else if self.stage == Stage::Locating && !net.fetch.busy() {
                    self.stage = Stage::Failed;
                }
            }
            Stage::Forecasting => {
                if let Some(text) = net.fetch.take() {
                    self.absorb_forecast(text);
                } else if net.fetch.take_failure() {
                    self.stage = Stage::Failed;
                }
            }
            Stage::Ready | Stage::Failed => {}
        }
    }

    fn absorb_forecast(&mut self, text: &str) {
        self.n = 0;
        // The four arrays are parallel and in a fixed order, so they are walked
        // together rather than indexed.
        let mut dates = json_array(text, "time");
        let mut codes = json_array(text, "weather_code");
        let mut highs = json_array(text, "temperature_2m_max");
        let mut lows = json_array(text, "temperature_2m_min");
        let mut rains = json_array(text, "precipitation_sum");
        while self.n < MAX_DAYS {
            let (Some(date), Some(code), Some(high), Some(low), Some(rain)) = (
                dates.next(),
                codes.next(),
                highs.next(),
                lows.next(),
                rains.next(),
            ) else {
                break;
            };
            let Some((weekday, month, dom)) = read_date(date) else {
                break;
            };
            self.days[self.n] = Day {
                weekday,
                dom,
                month,
                code: code.parse().unwrap_or(0),
                high: tenths(high).unwrap_or(0),
                low: tenths(low).unwrap_or(0),
                rain: tenths(rain).unwrap_or(0),
            };
            self.n += 1;
        }
        if self.n == 0 {
            self.stage = Stage::Failed;
            return;
        }
        self.day = 1.min(self.n - 1);
        self.stage = Stage::Ready;
    }

    pub fn draw(&self, scene: &mut Scene, alpha: u8) {
        scene.label(
            W as i32 / 2,
            54,
            FontId::Caption,
            DIM,
            alpha,
            Align::Center,
            if self.city.is_empty() {
                "WEATHER"
            } else {
                self.city.as_str()
            },
        );

        if self.stage != Stage::Ready {
            scene.label(
                W as i32 / 2,
                240,
                FontId::Body,
                if self.stage == Stage::Failed {
                    rgb(230, 110, 90)
                } else {
                    DIM
                },
                alpha,
                Align::Center,
                match self.stage {
                    Stage::Failed => "NO FORECAST",
                    Stage::Forecasting => "FORECAST\u{2026}",
                    _ => "LOCATING\u{2026}",
                },
            );
            return;
        }

        let day = self.days[self.day.min(self.n - 1)];
        let mut head = TextBuf::new();
        let _ = write!(
            head,
            "{} {} {}",
            weekday_name(day.weekday),
            day.dom,
            month_name(day.month)
        );
        scene.label(
            W as i32 / 2,
            108,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            head.as_str(),
        );

        draw_icon(scene, 128, 216, day.code, alpha);

        let mut high = TextBuf::new();
        let _ = write!(high, "{}\u{b0}", (day.high + 5) / 10);
        scene.label(
            W as i32 - 40,
            240,
            FontId::Display,
            INK,
            alpha,
            Align::Right,
            high.as_str(),
        );
        let mut low = TextBuf::new();
        let _ = write!(low, "NIGHT {}\u{b0}", (day.low + 5) / 10);
        scene.label(
            W as i32 - 40,
            282,
            FontId::Caption,
            DIM,
            alpha,
            Align::Right,
            low.as_str(),
        );

        // Rain, as a number and as a bar. Ten millimetres fills it, which is a
        // thoroughly wet day here - beyond that the number carries it.
        let mut rain = TextBuf::new();
        let whole = day.rain / 10;
        let frac = day.rain % 10;
        let _ = write!(rain, "RAIN {whole}.{frac} MM");
        scene.pill(40, 318, W as i32 - 40, 358, 20, rgb(22, 26, 32), alpha);
        let span = (day.rain.clamp(0, 100) * (W as i32 - 88)) / 100;
        if span > 8 {
            scene.pill(44, 322, 44 + span, 354, 16, muted(C_RAIN), alpha);
        }
        scene.label(
            W as i32 / 2,
            346,
            FontId::Caption,
            INK,
            alpha,
            Align::Center,
            rain.as_str(),
        );

        for (rect, text) in [(PREV, "PREV"), (NEXT, "NEXT")] {
            let (x0, y0, x1, y1) = rect;
            scene.pill(x0, y0, x1, y1, (y1 - y0) / 2, rgb(26, 30, 38), alpha);
            scene.label(
                (x0 + x1) / 2,
                (y0 + y1) / 2 + 9,
                FontId::Caption,
                DIM,
                alpha,
                Align::Center,
                text,
            );
        }
        for index in 0..self.n {
            let x = W as i32 / 2 - (self.n as i32 - 1) * 9 + index as i32 * 18;
            let here = index == self.day;
            scene.disc(
                x,
                432,
                if here { 6 } else { 4 },
                if here { C_SUN } else { rgb(52, 58, 68) },
                alpha,
            );
        }
    }
}

/// WMO code groups, as open-meteo documents them.
fn draw_icon(scene: &mut Scene, cx: i32, cy: i32, code: u16, alpha: u8) {
    let clear = code == 0;
    let sunny = code <= 2;
    let thunder = code >= 95;
    let snow = (71..=79).contains(&code) || code == 85 || code == 86;
    let wet = (51..=67).contains(&code) || (80..=82).contains(&code) || thunder;
    let fog = code == 45 || code == 48;

    if sunny {
        // Off to one side when there is cloud to sit behind.
        let (sx, sy) = if clear { (cx, cy) } else { (cx - 26, cy - 26) };
        scene.disc(sx, sy, 40, C_SUN, alpha);
        if clear {
            for step in 0..4 {
                let (dx, dy) = [(0, -62), (62, 0), (44, -44), (-44, -44)][step];
                scene.disc(sx + dx, sy + dy, 6, muted(C_SUN), alpha);
                scene.disc(sx - dx, sy - dy, 6, muted(C_SUN), alpha);
            }
            return;
        }
    }

    if fog {
        for row in 0..3 {
            let y = cy - 20 + row * 24;
            scene.pill(cx - 58, y, cx + 58, y + 12, 6, C_CLOUD, alpha);
        }
        return;
    }

    // Cloud: three lumps over a flat base.
    let color = if wet || snow { muted(C_CLOUD) } else { C_CLOUD };
    scene.disc(cx - 24, cy, 26, color, alpha);
    scene.disc(cx + 6, cy - 12, 34, color, alpha);
    scene.disc(cx + 34, cy + 2, 24, color, alpha);
    scene.pill(cx - 48, cy + 8, cx + 56, cy + 26, 9, color, alpha);

    if thunder {
        scene.pill(cx + 2, cy + 34, cx + 14, cy + 62, 5, C_BOLT, alpha);
        scene.pill(cx - 10, cy + 52, cx + 4, cy + 74, 5, C_BOLT, alpha);
        return;
    }
    if snow {
        for step in 0..3 {
            scene.disc(cx - 30 + step * 30, cy + 46 + (step % 2) * 10, 7, C_SNOW, alpha);
        }
        return;
    }
    if wet {
        for step in 0..3 {
            let x = cx - 32 + step * 30;
            let y = cy + 38 + (step % 2) * 10;
            scene.pill(x, y, x + 9, y + 22, 4, C_RAIN, alpha);
        }
    }
}

/// Weekday, month and day-of-month from an ISO `YYYY-MM-DD`.
///
/// Sakamoto's method for the weekday: a table of month offsets and two leap-year
/// corrections, which is the whole of the calendar arithmetic needed here.
fn read_date(date: &str) -> Option<(u8, u8, u8)> {
    let bytes = date.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    let year: i32 = date.get(0..4)?.parse().ok()?;
    let month: i32 = date.get(5..7)?.parse().ok()?;
    let dom: i32 = date.get(8..10)?.parse().ok()?;
    const OFFSET: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let sunday_first = (y + y / 4 - y / 100 + y / 400 + OFFSET[(month - 1) as usize] + dom) % 7;
    // Monday first, to match the names below.
    let weekday = (sunday_first + 6) % 7;
    Some((weekday as u8, month as u8, dom as u8))
}

fn weekday_name(weekday: u8) -> &'static str {
    [
        "MONDAY",
        "TUESDAY",
        "WEDNESDAY",
        "THURSDAY",
        "FRIDAY",
        "SATURDAY",
        "SUNDAY",
    ][(weekday % 7) as usize]
}

fn month_name(month: u8) -> &'static str {
    [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ][(month.max(1) - 1).min(11) as usize]
}
