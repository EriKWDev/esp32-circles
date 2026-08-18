//! CST9220 capacitive touch, with press/drag/release tracking.
//!
//! The demo only needed "a contact happened here", so it read the controller on
//! its interrupt edge and treated every report as a fresh press. A slider needs
//! continuous position and a real release, and the controller does not give a
//! stable down/up state - reports arrive as pulses while a finger moves, and
//! simply stop when it lifts.
//!
//! So this polls every frame and infers the gesture: any valid report means
//! "down here", and going quiet for RELEASE_MS means "lifted". That timeout is
//! the one tunable that matters - too short and a slow drag stutters into
//! separate presses, too long and the UI feels like it sticks to your finger.

use esp_hal::i2c::master::I2c;

const ADDR: u8 = 0x5a;
/// Silence longer than this counts as a lift. The panel reports at well under
/// 50 ms intervals while a finger is actually moving.
const RELEASE_MS: u32 = 70;
/// A press that never travels further than this stays a tap rather than a drag.
const TAP_SLOP: i32 = 18;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    /// Finger is down; `moved` distinguishes a drag from a tap in progress.
    Down { moved: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Event {
    None,
    /// Finger touched down at this point.
    Press(i32, i32),
    /// Finger moved while down.
    Drag(i32, i32),
    /// Finger lifted. `tap` is true when it never travelled beyond the slop.
    Release { x: i32, y: i32, tap: bool },
}

pub struct Touch {
    pub phase: Phase,
    pub x: i32,
    pub y: i32,
    origin_x: i32,
    origin_y: i32,
    last_report_ms: u32,
}

impl Touch {
    pub const fn new() -> Self {
        Self {
            phase: Phase::Idle,
            x: 0,
            y: 0,
            origin_x: 0,
            origin_y: 0,
            last_report_ms: 0,
        }
    }

    /// Read the controller and fold the result into a gesture. Call once a frame
    /// with a monotonic millisecond clock.
    pub fn poll(&mut self, i2c: &mut I2c<'_, esp_hal::Blocking>, now_ms: u32) -> Event {
        let contact = read_contact(i2c);

        if let Some((x, y)) = contact {
            self.last_report_ms = now_ms;
            match self.phase {
                Phase::Idle => {
                    self.phase = Phase::Down { moved: false };
                    self.x = x;
                    self.y = y;
                    self.origin_x = x;
                    self.origin_y = y;
                    return Event::Press(x, y);
                }
                Phase::Down { moved } => {
                    let travelled = (x - self.origin_x).abs().max((y - self.origin_y).abs());
                    self.x = x;
                    self.y = y;
                    let moved = moved || travelled > TAP_SLOP;
                    self.phase = Phase::Down { moved };
                    return Event::Drag(x, y);
                }
            }
        }

        if let Phase::Down { moved } = self.phase {
            if now_ms.wrapping_sub(self.last_report_ms) >= RELEASE_MS {
                self.phase = Phase::Idle;
                return Event::Release { x: self.x, y: self.y, tap: !moved };
            }
        }
        Event::None
    }
}

/// One decoded contact, or None when the controller has nothing to report.
///
/// The register layout and the x mirroring come from the vendor example: `d[6]`
/// is a frame marker, `d[5]` low bits are the contact count, and the low nibble
/// of `d[0]` identifies the report kind.
fn read_contact(i2c: &mut I2c<'_, esp_hal::Blocking>) -> Option<(i32, i32)> {
    let mut d = [0u8; 10];
    if i2c.write_read(ADDR, &[0xd0, 0x00], &mut d).is_err() {
        return None;
    }
    if d[6] != 0xab || d[5] & 0x7f == 0 || d[0] & 0x0f != 0x06 {
        return None;
    }
    let y = ((d[1] as u16) << 4) | ((d[3] as u16) >> 4);
    let raw_x = ((d[2] as u16) << 4) | ((d[3] as u16) & 0x0f);
    let x = 480u16.saturating_sub(raw_x);
    Some((
        (x as i32).clamp(0, crate::gfx::W as i32 - 1),
        (y as i32).clamp(0, crate::gfx::H as i32 - 1),
    ))
}
