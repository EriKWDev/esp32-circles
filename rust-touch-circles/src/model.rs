//! Irrigation controller state, and the parser for its `/api/dump` format.
//!
//! The controller answers one `key:value` per line with no JSON, and every
//! repeating record is a single fixed-arity line, so this parses with `split`
//! and `parse` only - no allocator, no serde, no intermediate buffers.
//!
//! Wall-clock time also comes from the dump (`time:`), which is the Axis
//! device's own local clock. That removes any need for NTP or an RTC here: the
//! clock is seeded from each poll and ticked locally in between.

use crate::gfx::Text;

pub const MAX_RELAYS: usize = 24;
pub const MAX_STARTS: usize = 12;
pub const MAX_ENTRIES: usize = 12;
pub const MAX_ANALOGS: usize = 24;
pub const MAX_CONTROLLERS: usize = 4;

#[derive(Clone, Copy)]
pub struct Relay {
    /// Consolidated id used by schedules and the UI.
    pub id: u8,
    /// Controller-local routing identity; deliberately not shown outside INFO.
    pub remote_id: u8,
    pub controller: u8,
    #[allow(dead_code)] // retained for wiring diagnostics / future INFO detail
    pub port: u8,
    pub enabled: bool,
    pub on: bool,
    pub name: Text,
}

impl Relay {
    const EMPTY: Relay = Relay {
        id: 0,
        remote_id: 0,
        controller: 0,
        port: 0,
        enabled: false,
        on: false,
        name: Text::EMPTY,
    };
}

#[derive(Clone, Copy)]
pub struct Entry {
    pub relay: u8,
    pub seconds: u16,
}

#[derive(Clone, Copy)]
pub struct StartTime {
    pub id: u8,
    pub enabled: bool,
    pub hh: u8,
    pub mm: u8,
    pub entries: [Entry; MAX_ENTRIES],
    pub n_entries: usize,
}

impl StartTime {
    const EMPTY: StartTime = StartTime {
        id: 0,
        enabled: false,
        hh: 0,
        mm: 0,
        entries: [Entry {
            relay: 0,
            seconds: 0,
        }; MAX_ENTRIES],
        n_entries: 0,
    };

    /// Total watering time this start time represents.
    pub fn total_seconds(&self) -> u32 {
        self.entries[..self.n_entries]
            .iter()
            .map(|e| e.seconds as u32)
            .sum()
    }
}

#[derive(Clone, Copy)]
pub struct Analog {
    #[allow(dead_code)] // stable hardware identity even when names are edited
    pub port: u8,
    pub level: u16,
    pub name: Text,
}

impl Analog {
    const EMPTY: Analog = Analog {
        port: 0,
        level: 0,
        name: Text::EMPTY,
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Connecting,
    Online,
    Offline,
}

pub struct State {
    pub relays: [Relay; MAX_RELAYS],
    pub n_relays: usize,
    pub starts: [StartTime; MAX_STARTS],
    pub n_starts: usize,
    pub analogs: [Analog; MAX_ANALOGS],
    pub n_analogs: usize,
    pub local_ip: Option<[u8; 4]>,
    pub controller_ips: [[u8; 4]; MAX_CONTROLLERS],
    pub controller_online: [bool; MAX_CONTROLLERS],
    pub n_controllers: usize,

    /// Controller-reported wall clock, ticked locally between polls.
    pub hh: u8,
    pub mm: u8,
    pub ss: u8,
    /// Millisecond accumulator for the local tick.
    pub clock_frac_ms: u32,
    pub clock_valid: bool,

    pub running: bool,
    /// Relay id, 0 when idle, -1 for a raw test pulse.
    pub active: i32,
    pub left_s: u32,
    pub queued: u32,
    pub max_run_s: u32,
    pub err: Text,
    pub link: Link,
}

impl State {
    pub const fn new() -> Self {
        Self {
            relays: [Relay::EMPTY; MAX_RELAYS],
            n_relays: 0,
            starts: [StartTime::EMPTY; MAX_STARTS],
            n_starts: 0,
            analogs: [Analog::EMPTY; MAX_ANALOGS],
            n_analogs: 0,
            local_ip: None,
            controller_ips: [[0; 4]; MAX_CONTROLLERS],
            controller_online: [false; MAX_CONTROLLERS],
            n_controllers: 0,
            hh: 0,
            mm: 0,
            ss: 0,
            clock_frac_ms: 0,
            clock_valid: false,
            running: false,
            active: 0,
            left_s: 0,
            queued: 0,
            max_run_s: 3600,
            err: Text::EMPTY,
            link: Link::Connecting,
        }
    }

    pub fn relay_by_id(&self, id: i32) -> Option<&Relay> {
        self.relays[..self.n_relays]
            .iter()
            .find(|r| r.id as i32 == id)
    }

    /// Relays that can actually be driven, in config order.
    pub fn usable(&self) -> impl Iterator<Item = &Relay> {
        self.relays[..self.n_relays].iter().filter(|r| r.enabled)
    }

    pub fn n_usable(&self) -> usize {
        self.usable().count()
    }

    /// Advance the local clock and the run countdown. `dt_ms` is real elapsed
    /// time, so this stays right even if a frame overruns.
    pub fn tick(&mut self, dt_ms: u32) {
        if !self.clock_valid {
            return;
        }
        self.clock_frac_ms += dt_ms;
        while self.clock_frac_ms >= 1000 {
            self.clock_frac_ms -= 1000;
            self.ss += 1;
            if self.ss >= 60 {
                self.ss = 0;
                self.mm += 1;
                if self.mm >= 60 {
                    self.mm = 0;
                    self.hh = (self.hh + 1) % 24;
                }
            }
            // Predict the countdown locally so the big number moves every
            // second rather than only when a poll lands.
            if self.running && self.left_s > 0 {
                self.left_s -= 1;
            }
        }
    }

    /// The next start time due after the current clock, and how many minutes
    /// away it is. Wraps to tomorrow when everything today has passed.
    pub fn next_start(&self) -> Option<(&StartTime, u32)> {
        if !self.clock_valid {
            return None;
        }
        let now = self.hh as u32 * 60 + self.mm as u32;
        let mut best: Option<(&StartTime, u32)> = None;
        for s in self.starts[..self.n_starts].iter() {
            if !s.enabled || s.n_entries == 0 {
                continue;
            }
            let at = s.hh as u32 * 60 + s.mm as u32;
            let delta = if at >= now {
                at - now
            } else {
                at + 24 * 60 - now
            };
            if best.is_none_or(|(_, d)| delta < d) {
                best = Some((s, delta));
            }
        }
        best
    }

    /// Time-derived progress for a schedule whose daily window contains the
    /// controller clock. Returns (elapsed, total, active entry, entry elapsed).
    /// This is intentionally independent of UI state, so it also works after a
    /// panel reboot; `run:` remains the authority for the persistent run badge.
    pub fn schedule_progress(&self, index: usize) -> Option<(u32, u32, usize, u32)> {
        let schedule = self.starts.get(index)?;
        if !self.clock_valid || !schedule.enabled || schedule.n_entries == 0 {
            return None;
        }
        let total = schedule.total_seconds();
        if total == 0 || total >= 24 * 60 * 60 {
            return None;
        }
        let now = self.hh as u32 * 3600 + self.mm as u32 * 60 + self.ss as u32;
        let start = schedule.hh as u32 * 3600 + schedule.mm as u32 * 60;
        let elapsed = (now + 24 * 60 * 60 - start) % (24 * 60 * 60);
        if elapsed >= total {
            return None;
        }
        let mut before = 0;
        for (entry, item) in schedule.entries[..schedule.n_entries].iter().enumerate() {
            let end = before + item.seconds as u32;
            if elapsed < end {
                return Some((elapsed, total, entry, elapsed - before));
            }
            before = end;
        }
        None
    }
}

/// Split "a:b:c" style records: returns the field at `index` of a line whose
/// fields are colon-separated, or None when it does not exist.
#[inline]
fn field(line: &str, index: usize) -> Option<&str> {
    line.split(':').nth(index)
}

fn num<T: core::str::FromStr>(line: &str, index: usize) -> Option<T> {
    field(line, index)?.parse().ok()
}

/// Everything after the `n`th colon, verbatim - the format puts free text
/// (names, error strings) last precisely so this works without escaping.
fn rest_after(line: &str, n: usize) -> &str {
    let mut seen = 0;
    for (i, b) in line.bytes().enumerate() {
        if b == b':' {
            seen += 1;
            if seen == n {
                return &line[i + 1..];
            }
        }
    }
    ""
}

/// Parse a whole `/api/dump` body into `state`.
///
/// Deliberately tolerant: unknown keys are ignored so this firmware keeps
/// working against a newer controller that adds records, which is exactly the
/// compatibility rule the dump format documents.
pub fn parse_dump(body: &str, state: &mut State) {
    let mut n_relays = 0;
    let mut n_starts = 0;
    let mut n_analogs = 0;
    let mut saw_time = false;

    for line in body.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(key) = line.split(':').next() else {
            continue;
        };
        match key {
            "r" => {
                if n_relays < MAX_RELAYS {
                    let id: u8 = num(line, 1).unwrap_or(0);
                    if id != 0 {
                        state.relays[n_relays] = Relay {
                            id,
                            remote_id: id,
                            controller: 0,
                            port: num(line, 2).unwrap_or(0),
                            enabled: num::<u8>(line, 3).unwrap_or(0) != 0,
                            on: num::<u8>(line, 4).unwrap_or(0) != 0,
                            name: Text::new(rest_after(line, 5)),
                        };
                        n_relays += 1;
                    }
                }
            }
            "s" => {
                if n_starts < MAX_STARTS {
                    state.starts[n_starts] = StartTime {
                        id: num(line, 1).unwrap_or(0),
                        enabled: num::<u8>(line, 2).unwrap_or(0) != 0,
                        hh: num(line, 3).unwrap_or(0),
                        mm: num(line, 4).unwrap_or(0),
                        entries: [Entry {
                            relay: 0,
                            seconds: 0,
                        }; MAX_ENTRIES],
                        n_entries: 0,
                    };
                    n_starts += 1;
                }
            }
            "e" => {
                let sid: u8 = num(line, 1).unwrap_or(0);
                let relay: u8 = num(line, 2).unwrap_or(0);
                let seconds: u16 = num(line, 3).unwrap_or(0);
                // The start id is repeated on every entry line, so entries can
                // be attached without tracking which s: line came before.
                if let Some(s) = state.starts[..n_starts].iter_mut().find(|s| s.id == sid)
                    && s.n_entries < MAX_ENTRIES
                {
                    s.entries[s.n_entries] = Entry { relay, seconds };
                    s.n_entries += 1;
                }
            }
            "a" => {
                if n_analogs < MAX_ANALOGS {
                    state.analogs[n_analogs] = Analog {
                        port: num(line, 1).unwrap_or(0),
                        level: num(line, 2).unwrap_or(0),
                        name: Text::new(rest_after(line, 3)),
                    };
                    n_analogs += 1;
                }
            }
            "time" => {
                // "time:HH:MM:SS" - the value itself contains colons.
                let hh = num::<u8>(line, 1);
                let mm = num::<u8>(line, 2);
                let ss = num::<u8>(line, 3);
                if let (Some(hh), Some(mm), Some(ss)) = (hh, mm, ss)
                    && hh < 24
                    && mm < 60
                    && ss < 60
                {
                    state.hh = hh;
                    state.mm = mm;
                    state.ss = ss;
                    state.clock_frac_ms = 0;
                    state.clock_valid = true;
                    saw_time = true;
                }
            }
            "run" => state.running = num::<u8>(line, 1).unwrap_or(0) != 0,
            "active" => state.active = num(line, 1).unwrap_or(0),
            "left" => state.left_s = num(line, 1).unwrap_or(0),
            "queued" => state.queued = num(line, 1).unwrap_or(0),
            "maxrun" => state.max_run_s = num(line, 1).unwrap_or(3600),
            "err" => state.err = Text::new(rest_after(line, 1)),
            _ => {}
        }
    }

    if n_relays > 0 {
        state.n_relays = n_relays;
    }
    if n_starts > 0 {
        state.n_starts = n_starts;
    }
    state.n_analogs = n_analogs;
    let _ = saw_time;
}
