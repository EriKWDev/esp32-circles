//! Rain override: don't water the day after a wet forecast.
//!
//! At 23:00 the panel asks what tomorrow looks like. More than five millimetres
//! and it disarms every schedule for the coming day, then re-arms them at the
//! next 23:00 once the forecast is dry again. At 11:00 it asks again, but only to
//! keep the symbol on the home screen honest - the decision is not revisited
//! mid-day, because half a day of watering already skipped cannot be un-skipped
//! and a forecast that changes its mind at lunchtime should not restart the
//! sprinklers.
//!
//! Disarming is done by posting the schedules back with their armed bit clear,
//! since the controller runs its own scheduler and there is nothing else that
//! would stop it. Which bits were set is remembered - in flash, not just in RAM,
//! because a reboot mid-override would otherwise leave the schedules off for
//! good, and that failure would be silent until a dry August.
//!
//! The clock comes from the controller and carries no date, so nothing here
//! tracks one. A day is simply the span between two 23:00 checks, which is
//! exactly the span the override covers.

use core::fmt::Write as _;

use crate::fetch::{json_array, scope, tenths};
use crate::weather::parse_location;
use crate::gfx::{Scene, muted, rgb};
use crate::model::State;
use crate::net::{Buf, Net};

/// Tenths of a millimetre over the day that count as "wet enough to skip".
const WET_TENTHS: i32 = 50;
const DECIDE_HOUR: u8 = 23;
/// Gap between attempts when a look does not come back.
const RETRY_MS: u32 = 60_000;
const REFRESH_HOUR: u8 = 11;
/// Only two days are needed - today, then tomorrow - and only two fields, which
/// keeps the reply small enough to be cheap to ask for twice a day.
const PATH_MET: &str = "/v1/forecast?latitude=";
const MET_TAIL: &str = "&daily=weather_code,precipitation_sum&timezone=auto&forecast_days=2";

const C_RAIN: u16 = rgb(96, 176, 255);
const C_SUN: u16 = rgb(255, 206, 84);
const C_CLOUD: u16 = rgb(176, 190, 205);

/// What the user asked for, overriding the forecast for one day.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Manual {
    /// Let the forecast decide.
    Auto,
    /// Water tomorrow whatever the sky says.
    Water,
    /// Skip tomorrow even if it is dry.
    Skip,
}

impl Manual {
    pub fn code(self) -> u8 {
        match self {
            Manual::Auto => 0,
            Manual::Water => 1,
            Manual::Skip => 2,
        }
    }

    pub fn from_code(code: u8) -> Self {
        match code {
            1 => Manual::Water,
            2 => Manual::Skip,
            _ => Manual::Auto,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Locating,
    Asking,
}

/// Which schedules were armed when the override took hold, keyed by controller
/// and the schedule's own id on it - a merged index would not survive a
/// controller dropping out and coming back in a different slot.
fn bit(controller: u8, remote_id: u8) -> u16 {
    let slot = controller as u32 * 3 + (remote_id.max(1) - 1) as u32;
    if slot < 16 { 1 << slot } else { 0 }
}

pub struct Rain {
    stage: Stage,
    lat: Buf<12>,
    lon: Buf<12>,
    /// Tomorrow, as of the last look.
    pub tomorrow_tenths: i32,
    pub tomorrow_code: u16,
    pub have_forecast: bool,
    /// Set while the schedules are disarmed by us.
    pub active: bool,
    /// What was armed before we disarmed it.
    armed_mask: u16,
    pub manual: Manual,
    /// The hour of the last check, so each of the two daily checks fires once.
    last_hour: Option<u8>,
    /// Schedules still to be posted, as (controller, remote_id) plus the armed
    /// bit to write. One request is in flight at a time.
    queue: [(u8, u8, bool); 8],
    n_queued: usize,
    /// True when something changed that flash should be told about.
    pub dirty: bool,
    /// Earliest a failed look may be tried again, so a server that is down is not
    /// asked once per frame.
    retry_after_ms: u32,
}

impl Rain {
    pub const fn new() -> Self {
        Self {
            stage: Stage::Idle,
            lat: Buf::new(),
            lon: Buf::new(),
            tomorrow_tenths: 0,
            tomorrow_code: 0,
            have_forecast: false,
            active: false,
            armed_mask: 0,
            manual: Manual::Auto,
            last_hour: None,
            queue: [(0, 0, false); 8],
            n_queued: 0,
            dirty: false,
            retry_after_ms: 0,
        }
    }

    pub fn restore(&mut self, active: bool, armed_mask: u16, manual: u8) {
        self.active = active;
        self.armed_mask = armed_mask;
        self.manual = Manual::from_code(manual);
    }

    /// Where the panel thinks it is, looked up at boot for the forecast. The
    /// flights page needs the same position and there is no reason to ask twice.
    pub fn location(&self) -> (&str, &str) {
        (self.lat.as_str(), self.lon.as_str())
    }

    pub fn saved(&self) -> (bool, u16, u8) {
        (self.active, self.armed_mask, self.manual.code())
    }

    /// Whether the home screen should say watering is off today.
    pub fn skipping(&self) -> bool {
        self.active
    }

    pub fn wet(&self) -> bool {
        self.have_forecast && self.tomorrow_tenths > WET_TENTHS
    }

    /// Water tomorrow regardless. Applied now as well as remembered, so pressing
    /// it during an override is also how you get today's watering back.
    pub fn choose_water(&mut self, state: &State) {
        self.manual = Manual::Water;
        self.dirty = true;
        if self.active {
            self.restore_arming(state);
        }
    }

    pub fn choose_skip(&mut self, state: &State) {
        self.manual = Manual::Skip;
        self.dirty = true;
        if !self.active {
            self.disarm(state);
        }
    }

    pub fn choose_auto(&mut self, state: &State) {
        self.manual = Manual::Auto;
        self.dirty = true;
        // Fall back to what the forecast says, immediately - otherwise "auto"
        // would appear to do nothing until 23:00.
        if self.wet() && !self.active {
            self.disarm(state);
        } else if !self.wet() && self.active {
            self.restore_arming(state);
        }
    }

    /// One step: the clock, then whatever fetch or post is outstanding.
    pub fn step(&mut self, net: &mut Net, state: &State, now_ms: u32) {
        self.pump_queue(net, state, now_ms);

        // One look shortly after boot, so the symbol is there from the start
        // rather than appearing at eleven.
        if !self.have_forecast && self.stage == Stage::Idle && state.clock_valid {
            self.begin(net, now_ms);
        }

        if state.clock_valid {
            let hour = state.hh;
            let due = hour == DECIDE_HOUR || hour == REFRESH_HOUR;
            if due && self.last_hour != Some(hour) && self.stage == Stage::Idle {
                self.last_hour = Some(hour);
                self.begin(net, now_ms);
            }
            // Cleared on any other hour so tomorrow's check fires again.
            if !due {
                self.last_hour = None;
            }
        }

        match self.stage {
            Stage::Idle => {}
            Stage::Locating => {
                let mut located = false;
                if let Some(text) = net.fetch.take() {
                    if let Some((_, lat, lon)) = parse_location(text) {
                        self.lat = Buf::new();
                        self.lon = Buf::new();
                        let _ = write!(self.lat, "{lat}");
                        let _ = write!(self.lon, "{lon}");
                        located = true;
                    }
                } else if net.fetch.take_failure() {
                    self.stage = Stage::Idle;
                }
                if located {
                    self.ask(net, now_ms);
                }
            }
            Stage::Asking => {
                if let Some(text) = net.fetch.take() {
                    self.absorb(text);
                    self.stage = Stage::Idle;
                    // Only the 23:00 look decides anything.
                    if state.hh == DECIDE_HOUR {
                        self.decide(state);
                    }
                } else if net.fetch.take_failure() {
                    self.stage = Stage::Idle;
                }
            }
        }
    }

    /// Ask for a fresh forecast now, from wherever the panel is.
    pub fn begin(&mut self, net: &mut Net, now_ms: u32) {
        if net.fetch.busy()
            || self.stage != Stage::Idle
            || now_ms.wrapping_sub(self.retry_after_ms) > u32::MAX / 2
        {
            return;
        }
        // Set before the request, not after it fails: a lookup that never answers
        // at all would otherwise never arm the throttle.
        self.retry_after_ms = now_ms + RETRY_MS;
        if self.lat.is_empty() {
            net.fetch_get(
                crate::weather::HOST_GEO,
                "/?fields=city,latitude,longitude",
                now_ms,
            );
            self.stage = Stage::Locating;
        } else {
            self.ask(net, now_ms);
        }
    }

    fn ask(&mut self, net: &mut Net, now_ms: u32) {
        let mut path = Buf::<160>::new();
        let _ = write!(
            path,
            "{PATH_MET}{}&longitude={}{MET_TAIL}",
            self.lat.as_str(),
            self.lon.as_str()
        );
        net.fetch_get(crate::weather::HOST_MET, path.as_str(), now_ms);
        self.stage = Stage::Asking;
    }

    fn absorb(&mut self, text: &str) {
        let daily = scope(text, "daily");
        // Index one is tomorrow: the request asked for two days starting today.
        let rain = json_array(daily, "precipitation_sum").nth(1).and_then(tenths);
        let code = json_array(daily, "weather_code")
            .nth(1)
            .and_then(|value| value.parse::<u16>().ok());
        if let (Some(rain), Some(code)) = (rain, code) {
            self.tomorrow_tenths = rain;
            self.tomorrow_code = code;
            self.have_forecast = true;
            esp_println::println!("rain: tomorrow {} tenths mm, code {}", rain, code);
        }
    }

    /// The 23:00 decision. A manual choice wins for the one day it was made for
    /// and is then spent, so tomorrow is the forecast's again.
    fn decide(&mut self, state: &State) {
        let skip = match self.manual {
            Manual::Water => false,
            Manual::Skip => true,
            Manual::Auto => self.wet(),
        };
        self.manual = Manual::Auto;
        self.dirty = true;
        match (skip, self.active) {
            (true, false) => self.disarm(state),
            (false, true) => self.restore_arming(state),
            _ => {}
        }
    }

    /// Remember what is armed, then queue every armed schedule to be posted back
    /// disarmed.
    fn disarm(&mut self, state: &State) {
        self.armed_mask = 0;
        self.n_queued = 0;
        for schedule in state.starts[..state.n_starts].iter() {
            if !schedule.enabled {
                continue;
            }
            self.armed_mask |= bit(schedule.controller, schedule.remote_id);
            self.push(schedule.controller, schedule.remote_id, false);
        }
        self.active = true;
        self.dirty = true;
        esp_println::println!("rain: disarming {} schedules", self.n_queued);
    }

    fn restore_arming(&mut self, state: &State) {
        self.n_queued = 0;
        for schedule in state.starts[..state.n_starts].iter() {
            if self.armed_mask & bit(schedule.controller, schedule.remote_id) == 0 {
                continue;
            }
            self.push(schedule.controller, schedule.remote_id, true);
        }
        self.active = false;
        self.armed_mask = 0;
        self.dirty = true;
        esp_println::println!("rain: re-arming {} schedules", self.n_queued);
    }

    fn push(&mut self, controller: u8, remote_id: u8, armed: bool) {
        if self.n_queued < self.queue.len() {
            self.queue[self.n_queued] = (controller, remote_id, armed);
            self.n_queued += 1;
        }
    }

    /// One post per pass, and only while nothing else is talking to a
    /// controller: these writes are never urgent, and a poll in flight is.
    fn pump_queue(&mut self, net: &mut Net, state: &State, now_ms: u32) {
        if self.n_queued == 0 || net.busy() {
            return;
        }
        let (controller, remote_id, armed) = self.queue[0];
        self.queue.rotate_left(1);
        self.n_queued -= 1;
        let Some(schedule) = state.starts[..state.n_starts]
            .iter()
            .find(|s| s.controller == controller && s.remote_id == remote_id)
        else {
            return;
        };
        // Posted whole, entries and all, because that is the only shape the
        // endpoint takes - only the armed bit differs from what is there.
        let mut edited = *schedule;
        edited.enabled = armed;
        net.post_schedule(&edited, state, now_ms);
    }

    /// A small weather symbol, for beside the clock. Same WMO groups as the
    /// weather app, drawn at a size that sits next to type.
    pub fn symbol(&self, scene: &mut Scene, cx: i32, cy: i32, alpha: u8) {
        let code = self.tomorrow_code;
        let wet = self.wet();
        if !self.have_forecast {
            scene.ring(cx, cy, 15, 12, muted(C_CLOUD), alpha);
            return;
        }
        if code <= 1 && !wet {
            scene.disc(cx, cy, 13, C_SUN, alpha);
            for (dx, dy) in [(0, -20), (20, 0), (14, -14), (-14, -14)] {
                scene.disc(cx + dx, cy + dy, 3, muted(C_SUN), alpha);
                scene.disc(cx - dx, cy - dy, 3, muted(C_SUN), alpha);
            }
            return;
        }
        let cloud = if wet { muted(C_CLOUD) } else { C_CLOUD };
        scene.disc(cx - 9, cy - 2, 10, cloud, alpha);
        scene.disc(cx + 4, cy - 7, 12, cloud, alpha);
        scene.pill(cx - 18, cy + 1, cx + 17, cy + 8, 4, cloud, alpha);
        if wet {
            for step in 0..3 {
                let x = cx - 12 + step * 11;
                scene.pill(x, cy + 12, x + 4, cy + 22, 2, C_RAIN, alpha);
            }
        }
    }
}
