//! Asteroids. Touch to aim; the ship flies at your finger and fires forward.
//!
//! No rotation anywhere, because the renderer has no rotated primitive - and it
//! turns out not to need one. Aim is a vector from the ship to the last touch,
//! normalised with an integer square root; the ship is a disc with a smaller nose
//! disc offset along that vector, and rocks are rings, which look like rocks and
//! cost two spans a scanline.

use crate::bubbles::Bubbles;
use crate::font::FontId;
use crate::gfx::{Align, H, Scene, TextBuf, W, muted, rgb};

const MAX_ROCKS: usize = 10;
const MAX_SHOTS: usize = 4;

const Q: i32 = 8;
const SHIP_R: i32 = 13;
const SHOT_R: i32 = 3;
const SHOT_SPEED: i32 = 380 * Q;
const SHOT_LIFE_MS: u32 = 1_100;
const THRUST: i32 = 620;
const DRAG_PER_S: i32 = 55;
const SHIP_SPEED_MAX: i32 = 210 * Q;
const FIRE_EVERY_MS: u32 = 260;
/// Radii, largest to smallest. A rock splits into the next size down and vanishes
/// at the end of the list.
const SIZES: [i32; 3] = [40, 25, 14];
const LIVES: u8 = 3;
/// Grace after a collision, so a rock sitting on the spawn point cannot take
/// every life at once.
const SPAWN_GRACE_MS: u32 = 1_200;
/// How far from the centre a wave appears: clear of the ship, comfortably inside
/// the screen.
const SPAWN_RING: i32 = 130;
/// Eight unit directions, as hundredths - a circle without trigonometry.
const RING: [(i32, i32); 8] = [
    (0, -100),
    (71, -71),
    (100, 0),
    (71, 71),
    (0, 100),
    (-71, 71),
    (-100, 0),
    (-71, -71),
];

pub struct Palette {
    pub rocks: [u16; 3],
    pub ship: u16,
}

const PALETTES: [Palette; 6] = [
    Palette {
        rocks: [rgb(150, 190, 230), rgb(120, 210, 235), rgb(180, 220, 250)],
        ship: rgb(220, 240, 255),
    },
    Palette {
        rocks: [rgb(235, 170, 110), rgb(240, 130, 90), rgb(250, 200, 140)],
        ship: rgb(255, 230, 190),
    },
    Palette {
        rocks: [rgb(130, 220, 170), rgb(90, 210, 200), rgb(170, 235, 190)],
        ship: rgb(215, 255, 230),
    },
    Palette {
        rocks: [rgb(220, 150, 220), rgb(190, 140, 240), rgb(240, 180, 240)],
        ship: rgb(250, 220, 250),
    },
    Palette {
        rocks: [rgb(225, 215, 130), rgb(200, 225, 120), rgb(240, 230, 170)],
        ship: rgb(250, 245, 200),
    },
    Palette {
        rocks: [rgb(190, 200, 215), rgb(150, 170, 190), rgb(215, 225, 240)],
        ship: rgb(240, 245, 252),
    },
];

#[derive(Clone, Copy)]
struct Rock {
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
    /// Index into SIZES; live when `Some`.
    size: Option<usize>,
}

const NO_ROCK: Rock = Rock {
    x: 0,
    y: 0,
    vx: 0,
    vy: 0,
    size: None,
};

#[derive(Clone, Copy)]
struct Shot {
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
    born_ms: u32,
    live: bool,
}

const NO_SHOT: Shot = Shot {
    x: 0,
    y: 0,
    vx: 0,
    vy: 0,
    born_ms: 0,
    live: false,
};

pub struct Asteroids {
    rocks: [Rock; MAX_ROCKS],
    shots: [Shot; MAX_SHOTS],
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
    /// Unit aim, Q. Also where the nose is drawn.
    aim_x: i32,
    aim_y: i32,
    /// Where the finger is; None means coast.
    target: Option<(i32, i32)>,
    next_fire_ms: u32,
    grace_until_ms: u32,
    pub level: u32,
    lives: u8,
    score: u32,
    /// Drawn or not this frame; the ship blinks while it cannot be hit.
    blink: bool,
}

impl Asteroids {
    pub const fn new() -> Self {
        Self {
            rocks: [NO_ROCK; MAX_ROCKS],
            shots: [NO_SHOT; MAX_SHOTS],
            x: (W as i32 / 2) * Q,
            y: (H as i32 / 2) * Q,
            vx: 0,
            vy: 0,
            aim_x: 0,
            aim_y: -Q,
            target: None,
            next_fire_ms: 0,
            grace_until_ms: 0,
            level: 0,
            lives: LIVES,
            score: 0,
            blink: true,
        }
    }

    pub fn palette(&self) -> &'static Palette {
        &PALETTES[self.level as usize % PALETTES.len()]
    }

    pub fn restart(&mut self, now_ms: u32) {
        *self = Self::new();
        self.spawn_wave(now_ms);
    }

    /// Four rocks, plus one per level, on a ring around the ship.
    ///
    /// Not on the screen border, which is where these started: a rock sitting on
    /// the edge is half wrapped around it and awkward to hit, so they begin well
    /// inside the play area but clear of the middle, where the ship is.
    fn spawn_wave(&mut self, now_ms: u32) {
        self.rocks = [NO_ROCK; MAX_ROCKS];
        let count = (4 + self.level as usize).min(MAX_ROCKS);
        for index in 0..count {
            let (cx, cy) = (W as i32 / 2, H as i32 / 2);
            // Eight points around the ring, so a wave is spread rather than
            // clustered, with the clock choosing where the pattern starts.
            let step = index as i32 + (now_ms as i32 / 97);
            let (ox, oy) = RING[(step as usize) % RING.len()];
            let seed = now_ms as i32 / 3 + index as i32 * 37;
            let radius = SPAWN_RING + (seed % 3) * 18;
            self.rocks[index] = Rock {
                x: (cx + ox * radius / 100) * Q,
                y: (cy + oy * radius / 100) * Q,
                // Drift across the screen rather than creeping: the first version
                // worked out to about six pixels a second.
                vx: (((seed % 5) - 2) * 20 + 12) * Q,
                vy: ((((seed / 5) % 5) - 2) * 20 - 12) * Q,
                size: Some(0),
            };
        }
        self.x = (W as i32 / 2) * Q;
        self.y = (H as i32 / 2) * Q;
        self.vx = 0;
        self.vy = 0;
        self.grace_until_ms = now_ms + SPAWN_GRACE_MS;
    }

    pub fn touch(&mut self, x: i32, y: i32) {
        self.target = Some((x, y));
    }

    pub fn release(&mut self) {
        self.target = None;
    }

    pub fn update(&mut self, dt_ms: u32, now_ms: u32, bubbles: &mut Bubbles) {
        let dt = dt_ms.min(40) as i32;

        // Aim and thrust toward the finger. The aim vector is kept normalised so
        // the nose and the shots agree regardless of how far away the touch is.
        if let Some((tx, ty)) = self.target {
            let (dx, dy) = (tx * Q - self.x, ty * Q - self.y);
            let len = isqrt((dx * dx + dy * dy) as u32) as i32;
            if len > Q {
                self.aim_x = dx * Q / len;
                self.aim_y = dy * Q / len;
                self.vx += self.aim_x * THRUST * dt / 1000 / Q;
                self.vy += self.aim_y * THRUST * dt / 1000 / Q;
            }
        }
        // Drag, so letting go coasts to a stop rather than drifting forever.
        self.vx -= self.vx * DRAG_PER_S * dt / 1000 / 100;
        self.vy -= self.vy * DRAG_PER_S * dt / 1000 / 100;
        let speed = isqrt((self.vx * self.vx + self.vy * self.vy) as u32) as i32;
        if speed > SHIP_SPEED_MAX {
            self.vx = self.vx * SHIP_SPEED_MAX / speed;
            self.vy = self.vy * SHIP_SPEED_MAX / speed;
        }
        self.x = wrap(self.x + self.vx * dt / 1000, W as i32);
        self.y = wrap(self.y + self.vy * dt / 1000, H as i32);

        if self.target.is_some() && now_ms >= self.next_fire_ms {
            self.next_fire_ms = now_ms + FIRE_EVERY_MS;
            if let Some(slot) = self.shots.iter_mut().find(|s| !s.live) {
                *slot = Shot {
                    x: self.x + self.aim_x * SHIP_R,
                    y: self.y + self.aim_y * SHIP_R,
                    vx: self.aim_x * SHOT_SPEED / Q,
                    vy: self.aim_y * SHOT_SPEED / Q,
                    born_ms: now_ms,
                    live: true,
                };
            }
        }

        for shot in self.shots.iter_mut().filter(|s| s.live) {
            shot.x = wrap(shot.x + shot.vx * dt / 1000, W as i32);
            shot.y = wrap(shot.y + shot.vy * dt / 1000, H as i32);
            if now_ms.wrapping_sub(shot.born_ms) > SHOT_LIFE_MS {
                shot.live = false;
            }
        }
        for rock in self.rocks.iter_mut().filter(|r| r.size.is_some()) {
            rock.x = wrap(rock.x + rock.vx * dt / 1000, W as i32);
            rock.y = wrap(rock.y + rock.vy * dt / 1000, H as i32);
        }

        // Blink through the grace period, so it is clear why rocks are passing
        // straight through.
        self.blink = now_ms >= self.grace_until_ms || (now_ms / 110) % 2 == 0;

        self.resolve(now_ms, bubbles);
    }

    fn resolve(&mut self, now_ms: u32, bubbles: &mut Bubbles) {
        let palette = self.palette();
        for si in 0..MAX_SHOTS {
            if !self.shots[si].live {
                continue;
            }
            for ri in 0..MAX_ROCKS {
                let Some(size) = self.rocks[ri].size else {
                    continue;
                };
                let r = SIZES[size];
                let dx = (self.shots[si].x - self.rocks[ri].x) / Q;
                let dy = (self.shots[si].y - self.rocks[ri].y) / Q;
                if dx * dx + dy * dy > (r + SHOT_R) * (r + SHOT_R) {
                    continue;
                }
                self.shots[si].live = false;
                self.score += 10 * (size as u32 + 1);
                let (rx, ry) = (self.rocks[ri].x, self.rocks[ri].y);
                bubbles.spawn(
                    rx / Q,
                    ry / Q,
                    now_ms,
                    Some(muted(palette.rocks[size])),
                    Some(r * 3),
                    true,
                );
                self.split(ri, size, now_ms);
                break;
            }
        }

        // The ship against a rock.
        if now_ms >= self.grace_until_ms {
            for ri in 0..MAX_ROCKS {
                let Some(size) = self.rocks[ri].size else {
                    continue;
                };
                let dx = (self.x - self.rocks[ri].x) / Q;
                let dy = (self.y - self.rocks[ri].y) / Q;
                let reach = SIZES[size] + SHIP_R;
                if dx * dx + dy * dy > reach * reach {
                    continue;
                }
                self.lives = self.lives.saturating_sub(1);
                bubbles.spawn(
                    self.x / Q,
                    self.y / Q,
                    now_ms,
                    Some(muted(rgb(240, 80, 70))),
                    Some(230),
                    true,
                );
                if self.lives == 0 {
                    let score = self.score;
                    self.restart(now_ms);
                    self.score = score;
                } else {
                    self.x = (W as i32 / 2) * Q;
                    self.y = (H as i32 / 2) * Q;
                    self.vx = 0;
                    self.vy = 0;
                    self.grace_until_ms = now_ms + SPAWN_GRACE_MS;
                }
                break;
            }
        }

        if !self.rocks.iter().any(|r| r.size.is_some()) {
            self.level += 1;
            self.spawn_wave(now_ms);
        }
    }

    /// Two smaller rocks, thrown off at right angles to the original drift - which
    /// needs no trigonometry, only a swap and a sign.
    fn split(&mut self, index: usize, size: usize, _now_ms: u32) {
        let parent = self.rocks[index];
        self.rocks[index].size = None;
        if size + 1 >= SIZES.len() {
            return;
        }
        let (px, py) = (parent.vy, -parent.vx);
        let pieces = [(px * 5 / 4, py * 5 / 4), (-px * 5 / 4, -py * 5 / 4)];
        for (vx, vy) in pieces {
            if let Some(slot) = self.rocks.iter_mut().find(|r| r.size.is_none()) {
                *slot = Rock {
                    x: parent.x,
                    y: parent.y,
                    vx: if vx == 0 { 40 } else { vx },
                    vy: if vy == 0 { 40 } else { vy },
                    size: Some(size + 1),
                };
            }
        }
    }

    pub fn draw(&self, scene: &mut Scene, alpha: u8) {
        let palette = self.palette();
        for rock in self.rocks.iter() {
            let Some(size) = rock.size else { continue };
            let r = SIZES[size];
            scene.ring(
                rock.x / Q,
                rock.y / Q,
                r,
                r - 4,
                palette.rocks[size],
                alpha,
            );
        }
        for shot in self.shots.iter().filter(|s| s.live) {
            scene.disc(shot.x / Q, shot.y / Q, SHOT_R, rgb(255, 255, 240), alpha);
        }

        if self.blink {
            scene.ring(self.x / Q, self.y / Q, SHIP_R, SHIP_R - 4, palette.ship, alpha);
            scene.disc(
                self.x / Q + self.aim_x * SHIP_R / Q,
                self.y / Q + self.aim_y * SHIP_R / Q,
                5,
                palette.ship,
                alpha,
            );
        }

        let mut line = TextBuf::new();
        use core::fmt::Write as _;
        let _ = write!(
            line,
            "WAVE {} \u{b7} {} \u{b7} {}",
            self.level + 1,
            self.score,
            self.lives
        );
        scene.label(
            W as i32 / 2,
            76,
            FontId::Caption,
            palette.rocks[0],
            alpha,
            Align::Center,
            line.as_str(),
        );
    }
}

/// Wrap a Q coordinate into the screen, so everything leaves one edge and returns
/// from the other.
fn wrap(value: i32, span: i32) -> i32 {
    value.rem_euclid(span * Q)
}

fn isqrt(n: u32) -> u32 {
    n.isqrt()
}
