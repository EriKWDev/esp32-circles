//! Spacewar!, 1962 - two ships, a star that pulls, and torpedoes.
//!
//! The star is the game: thrust is weak, gravity is not, and the fastest way
//! across the screen is a slingshot rather than a straight line. Fly into it and
//! you are gone. Space wraps, as it did on the PDP-1.
//!
//! This is the first game here that needs a heading, so it carries a sine table -
//! sixty-four directions, mirrored out of a sixteen-entry quarter, the same trick
//! the audio wavetable uses. The ships are still built from discs, because the
//! renderer has no rotated primitive: a body, and a nose offset along the heading.
//!
//! One contact at a time is the panel's real limitation, and it decides the
//! controls. Rotate is hold-to-turn and fire is a tap, but thrust *latches* - tap
//! it on, tap it off - because otherwise steering while under power would be
//! impossible rather than merely difficult, and Spacewar without that is not
//! Spacewar.

use crate::bubbles::Bubbles;
use crate::font::FontId;
use crate::gfx::{Align, H, Scene, TextBuf, W, muted, rgb};

/// Sub-pixel position and velocity.
const Q: i32 = 8;
/// Sixty-fourths of a turn.
const TURN: i32 = 64;
const MAX_SHOTS: usize = 6;

const STAR_R: i32 = 15;
/// Pull at one pixel's distance, in Q units per second per second. Tuned so an
/// orbit is findable but never quite stable.
const GRAVITY: i32 = 190_000;
/// Below this the pull is not computed - inside the star nothing survives anyway,
/// and the reciprocal would run away.
const GRAVITY_MIN_R: i32 = 14;
const SHIP_R: i32 = 11;
const THRUST: i32 = 300;
const SPEED_MAX: i32 = 200 * Q;
const SPIN_PER_S: i32 = 26;
const SHOT_R: i32 = 3;
const SHOT_SPEED: i32 = 260 * Q;
const SHOT_LIFE_MS: u32 = 1_500;
const FIRE_EVERY_MS: u32 = 320;
const RESPAWN_MS: u32 = 1_300;

pub const P1: u16 = rgb(90, 170, 255);
pub const P2: u16 = rgb(255, 150, 60);
const C_STAR: u16 = rgb(255, 220, 120);
const INK: u16 = rgb(238, 245, 250);
const PLATE: u16 = rgb(26, 30, 38);

/// A quarter of a sine in Q12, mirrored into the whole circle below.
const SIN: [i32; TURN as usize] = {
    let quarter: [i32; 17] = [
        0, 401, 799, 1189, 1567, 1931, 2276, 2598, 2896, 3166, 3406, 3612, 3784, 3920, 4017, 4076,
        4096,
    ];
    let mut table = [0i32; TURN as usize];
    let mut i = 0;
    while i <= 16 {
        table[i] = quarter[i];
        table[32 - i] = quarter[i];
        table[(32 + i) % 64] = -quarter[i];
        table[(64 - i) % 64] = -quarter[i];
        i += 1;
    }
    table
};

fn sin(turn: i32) -> i32 {
    SIN[turn.rem_euclid(TURN) as usize]
}

fn cos(turn: i32) -> i32 {
    sin(turn + TURN / 4)
}

/// Which control a contact landed on. Each player has the same four.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Left(u8),
    Right(u8),
    Thrust(u8),
    Fire(u8),
}

#[derive(Clone, Copy)]
struct Ship {
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
    heading: i32,
    thrusting: bool,
    /// Which way it is being turned this frame, cleared on release.
    spin: i32,
    dead_until_ms: u32,
    last_fire_ms: u32,
    score: u8,
}

impl Ship {
    const fn new(x: i32, y: i32, heading: i32) -> Self {
        Self {
            x: x * Q,
            y: y * Q,
            vx: 0,
            vy: 0,
            heading,
            thrusting: false,
            spin: 0,
            dead_until_ms: 0,
            last_fire_ms: 0,
            score: 0,
        }
    }

    fn alive(&self, now_ms: u32) -> bool {
        now_ms >= self.dead_until_ms
    }
}

#[derive(Clone, Copy)]
struct Shot {
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
    until_ms: u32,
    owner: u8,
}

pub struct Spacewar {
    ships: [Ship; 2],
    shots: [Shot; MAX_SHOTS],
    /// The control currently under the one contact the panel reports.
    held: Option<Control>,
}

const NO_SHOT: Shot = Shot {
    x: 0,
    y: 0,
    vx: 0,
    vy: 0,
    until_ms: 0,
    owner: 0,
};

impl Spacewar {
    pub const fn new() -> Self {
        Self {
            // Facing across the star at each other, as the original set up.
            ships: [
                Ship::new(96, H as i32 - 96, TURN / 8 * 7),
                Ship::new(W as i32 - 96, 96, TURN / 8 * 3),
            ],
            shots: [NO_SHOT; MAX_SHOTS],
            held: None,
        }
    }

    pub fn restart(&mut self) {
        *self = Self::new();
    }

    /// The four controls per player, as circles. P1 sits bottom-left and P2
    /// top-right: the panel lies flat between two people facing each other, so
    /// each cluster falls under the nearer hand.
    pub fn buttons() -> [(Control, i32, i32, i32); 8] {
        const R: i32 = 30;
        const NEAR: i32 = 46;
        const FAR: i32 = 112;
        let bottom = H as i32 - NEAR;
        let right = W as i32 - NEAR;
        [
            (Control::Left(0), NEAR, bottom, R),
            (Control::Right(0), NEAR + 66, bottom, R),
            (Control::Thrust(0), NEAR, H as i32 - FAR, R),
            (Control::Fire(0), NEAR + 66, H as i32 - FAR, R),
            (Control::Left(1), right, NEAR, R),
            (Control::Right(1), right - 66, NEAR, R),
            (Control::Thrust(1), right, FAR, R),
            (Control::Fire(1), right - 66, FAR, R),
        ]
    }

    pub fn control_at(x: i32, y: i32) -> Option<Control> {
        Self::buttons().into_iter().find_map(|(control, cx, cy, r)| {
            let (dx, dy) = (x - cx, y - cy);
            (dx * dx + dy * dy <= (r + 6) * (r + 6)).then_some(control)
        })
    }

    pub fn press(&mut self, x: i32, y: i32, now_ms: u32) {
        let Some(control) = Self::control_at(x, y) else {
            return;
        };
        self.held = Some(control);
        match control {
            // Both of these act once per contact; rotation is what a hold is for.
            Control::Thrust(who) => {
                let ship = &mut self.ships[who as usize];
                ship.thrusting = !ship.thrusting;
            }
            Control::Fire(who) => self.fire(who, now_ms),
            _ => {}
        }
    }

    /// A finger that slides off its button stops turning; one that slides onto
    /// another starts turning that way. Thrust and fire do not re-trigger.
    pub fn drag(&mut self, x: i32, y: i32) {
        match Self::control_at(x, y) {
            Some(control @ (Control::Left(_) | Control::Right(_))) => self.held = Some(control),
            _ => self.held = None,
        }
    }

    pub fn release(&mut self) {
        self.held = None;
    }

    fn fire(&mut self, who: u8, now_ms: u32) {
        let ship = self.ships[who as usize];
        if !ship.alive(now_ms) || now_ms.wrapping_sub(ship.last_fire_ms) < FIRE_EVERY_MS {
            return;
        }
        let Some(slot) = self.shots.iter().position(|shot| shot.until_ms <= now_ms) else {
            return;
        };
        let (sx, sy) = (sin(ship.heading), -cos(ship.heading));
        self.shots[slot] = Shot {
            // Clear of the nose, or a ship shoots itself on the first frame.
            x: ship.x + sx * (SHIP_R + 8) * Q / 4096,
            y: ship.y + sy * (SHIP_R + 8) * Q / 4096,
            vx: ship.vx + sx * SHOT_SPEED / 4096,
            vy: ship.vy + sy * SHOT_SPEED / 4096,
            until_ms: now_ms + SHOT_LIFE_MS,
            owner: who,
        };
        self.ships[who as usize].last_fire_ms = now_ms;
    }

    pub fn update(&mut self, dt_ms: u32, now_ms: u32, bubbles: &mut Bubbles) {
        let dt = dt_ms.min(40) as i32;
        let (cx, cy) = ((W as i32 / 2) * Q, (H as i32 / 2) * Q);

        // Rotation comes from whichever button is held, this frame only.
        for ship in self.ships.iter_mut() {
            ship.spin = 0;
        }
        match self.held {
            Some(Control::Left(who)) => self.ships[who as usize].spin = -1,
            Some(Control::Right(who)) => self.ships[who as usize].spin = 1,
            _ => {}
        }

        for index in 0..2 {
            let mut ship = self.ships[index];
            if !ship.alive(now_ms) {
                self.ships[index] = ship;
                continue;
            }
            // Turn in whole sixty-fourths, accumulated over time rather than per
            // frame, so the rate does not depend on the frame rate.
            if ship.spin != 0 {
                let steps = SPIN_PER_S * dt / 1000;
                ship.heading = (ship.heading + ship.spin * steps.max(1)).rem_euclid(TURN);
            }
            if ship.thrusting {
                ship.vx += sin(ship.heading) * THRUST * dt / 1000 / 4096;
                ship.vy += -cos(ship.heading) * THRUST * dt / 1000 / 4096;
            }
            let (vx, vy) = gravity(ship.x - cx, ship.y - cy, dt);
            ship.vx = (ship.vx + vx).clamp(-SPEED_MAX, SPEED_MAX);
            ship.vy = (ship.vy + vy).clamp(-SPEED_MAX, SPEED_MAX);
            ship.x = wrap(ship.x + ship.vx * dt / 1000, W as i32);
            ship.y = wrap(ship.y + ship.vy * dt / 1000, H as i32);

            // Into the star.
            if near(ship.x - cx, ship.y - cy, STAR_R + SHIP_R - 4) {
                self.kill(index, now_ms, bubbles);
                continue;
            }
            self.ships[index] = ship;
        }

        // Ships into each other: both go, and neither scores.
        if self.ships[0].alive(now_ms)
            && self.ships[1].alive(now_ms)
            && near(
                self.ships[0].x - self.ships[1].x,
                self.ships[0].y - self.ships[1].y,
                SHIP_R * 2 - 4,
            )
        {
            self.kill(0, now_ms, bubbles);
            self.kill(1, now_ms, bubbles);
        }

        for slot in 0..MAX_SHOTS {
            let mut shot = self.shots[slot];
            if shot.until_ms <= now_ms {
                continue;
            }
            // Torpedoes ignore the star's pull, as they did: a shot that curved
            // would make the slingshot unreadable.
            shot.x = wrap(shot.x + shot.vx * dt / 1000, W as i32);
            shot.y = wrap(shot.y + shot.vy * dt / 1000, H as i32);
            if near(shot.x - cx, shot.y - cy, STAR_R) {
                self.shots[slot] = NO_SHOT;
                continue;
            }
            self.shots[slot] = shot;

            let target = 1 - shot.owner as usize;
            if self.ships[target].alive(now_ms)
                && near(
                    shot.x - self.ships[target].x,
                    shot.y - self.ships[target].y,
                    SHIP_R + SHOT_R,
                )
            {
                self.shots[slot] = NO_SHOT;
                self.ships[shot.owner as usize].score =
                    self.ships[shot.owner as usize].score.saturating_add(1);
                self.kill(target, now_ms, bubbles);
            }
        }
    }

    /// A kill respawns the ship where it started, still and facing inward, after
    /// a pause - the alternative is dying twice to the same torpedo.
    fn kill(&mut self, index: usize, now_ms: u32, bubbles: &mut Bubbles) {
        let ship = &mut self.ships[index];
        bubbles.spawn(
            ship.x / Q,
            ship.y / Q,
            now_ms,
            Some(muted(if index == 0 { P1 } else { P2 })),
            None,
            true,
        );
        let score = ship.score;
        let fresh = Self::new().ships[index];
        *ship = Ship { score, ..fresh };
        ship.dead_until_ms = now_ms + RESPAWN_MS;
    }

    pub fn draw(&self, scene: &mut Scene, now_ms: u32, alpha: u8) {
        let (cx, cy) = (W as i32 / 2, H as i32 / 2);
        // The star pulses, which is the only animation it needs to look dangerous.
        let pulse = (now_ms / 90 % 6) as i32;
        scene.ring(cx, cy, STAR_R + 8 + pulse, STAR_R + 4, muted(C_STAR), alpha);
        scene.disc(cx, cy, STAR_R, C_STAR, alpha);

        for (index, ship) in self.ships.iter().enumerate() {
            let color = if index == 0 { P1 } else { P2 };
            if !ship.alive(now_ms) {
                continue;
            }
            let (x, y) = (ship.x / Q, ship.y / Q);
            scene.disc(x, y, SHIP_R, color, alpha);
            let nose = |distance: i32| {
                (
                    x + sin(ship.heading) * distance / 4096,
                    y - cos(ship.heading) * distance / 4096,
                )
            };
            let (nx, ny) = nose(SHIP_R + 4);
            scene.disc(nx, ny, 6, INK, alpha);
            if ship.thrusting {
                let (ex, ey) = nose(-SHIP_R - 6);
                scene.disc(ex, ey, 5, muted(C_STAR), alpha);
            }
        }

        for shot in self.shots.iter().filter(|shot| shot.until_ms > now_ms) {
            scene.disc(
                shot.x / Q,
                shot.y / Q,
                SHOT_R,
                if shot.owner == 0 { P1 } else { P2 },
                alpha,
            );
        }

        for (control, bx, by, r) in Self::buttons() {
            let who = match control {
                Control::Left(who) | Control::Right(who) => who,
                Control::Thrust(who) | Control::Fire(who) => who,
            };
            let color = if who == 0 { P1 } else { P2 };
            let latched = matches!(control, Control::Thrust(w) if self.ships[w as usize].thrusting);
            let live = latched || self.held == Some(control);
            scene.disc(bx, by, r, if live { muted(color) } else { PLATE }, alpha);
            scene.label(
                bx,
                by + 9,
                FontId::Caption,
                if live { INK } else { color },
                alpha,
                Align::Center,
                match control {
                    // Turn arrows as chevrons; the font has no arrow glyphs.
                    Control::Left(_) => "<",
                    Control::Right(_) => ">",
                    Control::Thrust(_) => "^",
                    Control::Fire(_) => "O",
                },
            );
        }

        let mut score = TextBuf::new();
        use core::fmt::Write as _;
        let _ = write!(score, "{} - {}", self.ships[0].score, self.ships[1].score);
        scene.label(
            cx,
            58,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            score.as_str(),
        );
    }
}

/// Velocity change from the star over `dt`, as an inverse-square pull.
fn gravity(dx: i32, dy: i32, dt: i32) -> (i32, i32) {
    let distance = ((dx * dx + dy * dy) as u32).isqrt() as i32 / Q;
    if distance < GRAVITY_MIN_R {
        return (0, 0);
    }
    // Magnitude first, then split along the offset. Dividing by distance a third
    // time is what turns the offset into a direction.
    let pull = GRAVITY * dt / 1000 / (distance * distance);
    (
        -pull * (dx / Q) / distance,
        -pull * (dy / Q) / distance,
    )
}

fn near(dx: i32, dy: i32, radius: i32) -> bool {
    let r = radius * Q;
    dx * dx + dy * dy <= r * r
}

fn wrap(value: i32, span: i32) -> i32 {
    value.rem_euclid(span * Q)
}
