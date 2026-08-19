//! A cat. It drifts about, and tapping it pleases it; tapping elsewhere leaves a
//! treat, which it goes and eats.
//!
//! Built from the same primitives as everything else - discs for the head and
//! muzzle, pills for ears, tail and whiskers - so it costs about a dozen shapes
//! and needs no bitmap.

use crate::gfx::{H, Scene, W, rgb};

const Q: i32 = 8;
const HEAD_R: i32 = 66;
const DRIFT: i32 = 26 * Q;
const CHASE: i32 = 96 * Q;

pub struct Cat {
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
    treat: Option<(i32, i32)>,
    /// Counts down while it is pleased, which is what shuts its eyes.
    happy_ms: u32,
    blink_ms: u32,
    /// Drives the tail and the wander, so nothing is ever perfectly still.
    sway: u32,
}

impl Cat {
    pub const fn new() -> Self {
        Self {
            x: (W as i32 / 2) * Q,
            y: (H as i32 / 2) * Q,
            vx: DRIFT,
            vy: DRIFT / 2,
            treat: None,
            happy_ms: 0,
            blink_ms: 0,
            sway: 0,
        }
    }

    pub fn restart(&mut self) {
        *self = Self::new();
    }

    /// True when the tap landed on the cat, which the caller answers with a purr.
    pub fn tap(&mut self, x: i32, y: i32) -> bool {
        let (dx, dy) = (x - self.x / Q, y - self.y / Q);
        if dx * dx + dy * dy <= HEAD_R * HEAD_R {
            self.happy_ms = 900;
            true
        } else {
            self.treat = Some((x, y));
            false
        }
    }

    /// True on the frame a treat is eaten, so the caller can meow.
    pub fn update(&mut self, dt_ms: u32) -> bool {
        let dt = dt_ms.min(50) as i32;
        self.sway = self.sway.wrapping_add(dt_ms);
        self.happy_ms = self.happy_ms.saturating_sub(dt_ms);
        self.blink_ms = self.blink_ms.saturating_sub(dt_ms);
        if self.blink_ms == 0 && (self.sway / 97) % 41 == 0 {
            self.blink_ms = 160;
        }

        let mut ate = false;
        if let Some((tx, ty)) = self.treat {
            let (dx, dy) = (tx * Q - self.x, ty * Q - self.y);
            let distance = ((dx * dx + dy * dy) as u32).isqrt() as i32;
            if distance < 26 * Q {
                self.treat = None;
                self.happy_ms = 700;
                ate = true;
            } else {
                self.vx = dx * CHASE / distance.max(1) / Q;
                self.vy = dy * CHASE / distance.max(1) / Q;
            }
        } else {
            // Idle drift, turned gently so it wanders rather than bouncing round a
            // box.
            let nudge = ((self.sway / 23) % 3) as i32 - 1;
            self.vx += nudge;
            self.vy -= nudge;
            let speed = ((self.vx * self.vx + self.vy * self.vy) as u32).isqrt() as i32;
            if speed > DRIFT {
                self.vx = self.vx * DRIFT / speed;
                self.vy = self.vy * DRIFT / speed;
            }
        }

        self.x += self.vx * dt / 1000;
        self.y += self.vy * dt / 1000;
        // It turns at the edges rather than wrapping: something that leaves one
        // side and reappears at the other is a ghost, not a cat.
        let margin = HEAD_R + 12;
        if self.x < margin * Q {
            self.x = margin * Q;
            self.vx = self.vx.abs();
        }
        if self.x > (W as i32 - margin) * Q {
            self.x = (W as i32 - margin) * Q;
            self.vx = -self.vx.abs();
        }
        if self.y < (margin + 30) * Q {
            self.y = (margin + 30) * Q;
            self.vy = self.vy.abs();
        }
        if self.y > (H as i32 - margin) * Q {
            self.y = (H as i32 - margin) * Q;
            self.vy = -self.vy.abs();
        }
        ate
    }

    pub fn draw(&self, scene: &mut Scene, alpha: u8) {
        let (cx, cy) = (self.x / Q, self.y / Q);
        let fur = rgb(255, 190, 120);
        let dark = rgb(190, 130, 80);

        if let Some((tx, ty)) = self.treat {
            scene.disc(tx, ty, 12, rgb(255, 120, 170), alpha);
            scene.ring(tx, ty, 19, 16, rgb(255, 170, 200), alpha);
        }

        let swing = (((self.sway / 6) % 40) as i32 - 20) / 2;
        let tail = if self.vx > 0 { cx - HEAD_R - 10 } else { cx + HEAD_R - 4 };
        scene.pill(tail, cy + 20 + swing, tail + 24, cy + 32 + swing, 6, dark, alpha);

        for side in [-1, 1] {
            let ex = cx + side * (HEAD_R - 18);
            scene.pill(ex - 16, cy - HEAD_R - 14, ex + 16, cy - HEAD_R + 18, 12, dark, alpha);
        }
        scene.disc(cx, cy, HEAD_R, fur, alpha);

        let shut = self.happy_ms > 0 || self.blink_ms > 0;
        for side in [-1, 1] {
            let ex = cx + side * 24;
            if shut {
                scene.pill(ex - 12, cy - 10, ex + 12, cy - 5, 3, dark, alpha);
            } else {
                scene.disc(ex, cy - 8, 9, rgb(60, 45, 40), alpha);
                scene.disc(ex - 3, cy - 11, 3, rgb(255, 255, 255), alpha);
            }
        }

        scene.disc(cx, cy + 18, 22, rgb(255, 225, 190), alpha);
        scene.disc(cx, cy + 12, 6, rgb(230, 130, 140), alpha);
        if self.happy_ms > 0 {
            scene.pill(cx - 14, cy + 22, cx - 2, cy + 27, 3, dark, alpha);
            scene.pill(cx + 2, cy + 22, cx + 14, cy + 27, 3, dark, alpha);
        } else {
            scene.pill(cx - 8, cy + 22, cx + 8, cy + 26, 2, dark, alpha);
        }

        for side in [-1, 1] {
            for row in 0..2 {
                let y = cy + 14 + row * 9;
                let (a, b) = (cx + side * 24, cx + side * 58);
                scene.pill(a.min(b), y, a.max(b), y + 3, 1, rgb(240, 210, 180), alpha);
            }
        }
    }
}
