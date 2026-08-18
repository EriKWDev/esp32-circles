//! The circles demo this firmware grew out of, kept playable as its own page.
//!
//! This is the animation from the project's `main` branch: touch anywhere and an
//! opaque disc of the next palette colour grows from that point to the farthest
//! corner with a cubic ease, holds, then fades. Drag and you leave a trail of
//! them, because a new contact more than 28 px from any live circle's origin
//! starts another one.
//!
//! Ported rather than copied. On `main` the demo *is* the firmware, so it owns
//! the panel: it drives the CO5300's window and stripe commands itself, updates
//! single 16x32 tiles while one circle grows, and fades the last circle with the
//! hardware brightness register instead of touching pixels. None of that can
//! coexist with a UI that composites other screens, so the model - timings,
//! radii, palette, spawn rule, and the culling that keeps it cheap - is
//! reproduced exactly and drawn through the ordinary scanline compositor.
//!
//! Two deliberate differences, both forced by living inside the UI:
//!
//! - The fade is alpha rather than panel brightness. Dimming the backlight would
//!   dim the Back button with it, and this page keeps one.
//! - Growth is in whole pixels rather than Q4. At 60 fps a circle gains about
//!   8 px a frame, so sub-pixel radii were never visible in the first place.
//!
//! Cost is bounded by the same trick the original uses: a circle that has
//! finished growing and is still opaque covers the whole panel, so everything
//! older is discarded. The live count therefore stays small however hard the
//! panel is tapped. What remains is cheap because a fully opaque span takes
//! `fill_span`'s paired-store path - close to a memset - and only the fading
//! circle, which by that rule is alone, pays for per-pixel blending.

use crate::gfx::{H, Scene, W, rgb};

/// As on `main`. The culling below means this is a safety limit rather than a
/// number the animation normally approaches.
const MAX_CIRCLES: usize = 32;
const GROW_MS: u32 = 700;
const FADE_MS: u32 = 520;
/// A contact this close to a live circle's origin is a repeat report, not a new
/// press. The touch controller pulses while a finger is held, and this is what
/// turns that into "one circle per place you touch" - and, when you drag, into a
/// trail spaced by roughly a fingertip.
const SAME_ORIGIN_R2: i32 = 28 * 28;

const PALETTE: [u16; 8] = [
    rgb(255, 55, 125),
    rgb(80, 210, 255),
    rgb(255, 185, 45),
    rgb(135, 80, 255),
    rgb(45, 255, 170),
    rgb(255, 90, 45),
    rgb(70, 125, 255),
    rgb(240, 70, 235),
];

#[derive(Clone, Copy)]
struct Circle {
    x: i32,
    y: i32,
    born_ms: u32,
    /// Distance to the farthest corner: where the circle stops growing, having
    /// covered the panel.
    full_r: i32,
    color: u16,
}

const NOTHING: Circle = Circle {
    x: 0,
    y: 0,
    born_ms: 0,
    full_r: 0,
    color: 0,
};

pub struct Bubbles {
    circles: [Circle; MAX_CIRCLES],
    len: usize,
    next_color: usize,
}

impl Bubbles {
    pub const fn new() -> Self {
        Self {
            circles: [NOTHING; MAX_CIRCLES],
            len: 0,
            next_color: 0,
        }
    }

    /// Start a circle at a contact point. False when the point belongs to one
    /// that is already alive, or when the table is full.
    pub fn press(&mut self, x: i32, y: i32, now_ms: u32) -> bool {
        for circle in &self.circles[..self.len] {
            let (dx, dy) = (x - circle.x, y - circle.y);
            if dx * dx + dy * dy <= SAME_ORIGIN_R2 {
                return false;
            }
        }
        if self.len >= MAX_CIRCLES {
            return false;
        }
        let x = x.clamp(0, W as i32 - 1);
        let y = y.clamp(0, H as i32 - 1);
        let dx = x.max(W as i32 - 1 - x) as u32;
        let dy = y.max(H as i32 - 1 - y) as u32;
        self.circles[self.len] = Circle {
            x,
            y,
            born_ms: now_ms,
            full_r: (dx * dx + dy * dy).isqrt() as i32 + 2,
            color: PALETTE[self.next_color % PALETTE.len()],
        };
        self.len += 1;
        self.next_color += 1;
        true
    }

    /// Retire finished circles, and drop everything hidden behind a grown one.
    pub fn update(&mut self, now_ms: u32) {
        let mut write = 0;
        let mut newest_covering = None;
        for read in 0..self.len {
            let circle = self.circles[read];
            let (radius, alpha) = state(&circle, now_ms);
            if alpha == 0 {
                continue;
            }
            if radius >= circle.full_r {
                newest_covering = Some(write);
            }
            self.circles[write] = circle;
            write += 1;
        }
        self.len = write;
        // A circle at full radius spans the panel, so nothing older can be seen
        // through it. Discarding those is what keeps the live count - and the
        // frame cost - small no matter how much the screen is tapped.
        if let Some(first) = newest_covering {
            self.circles.copy_within(first..self.len, 0);
            self.len -= first;
        }
    }

    /// True while anything is still moving, so the loop knows to keep painting.
    pub fn active(&self) -> bool {
        self.len != 0
    }

    /// Oldest first, so the newest and smallest ends up on top - the layered look
    /// the original has.
    ///
    /// `screen_alpha` is the page transition's own fade, folded in rather than
    /// applied afterwards, so a circle mid-fade during a transition does not
    /// briefly become more opaque than the screen carrying it.
    pub fn draw(&self, scene: &mut Scene, now_ms: u32, screen_alpha: u8) {
        for circle in &self.circles[..self.len] {
            let (radius, alpha) = state(circle, now_ms);
            if alpha == 0 || radius <= 0 {
                continue;
            }
            let alpha = ((alpha as u32 * screen_alpha as u32) / 255) as u8;
            // The occluding batch, not an ordinary primitive: overlapping
            // screen-filling discs are what this page is made of, and painting
            // those back-to-front costs a full-screen fill each. See `discs_row`.
            scene.push_disc(circle.x, circle.y, radius, circle.color, alpha);
        }
    }
}

/// Radius and opacity at `now_ms`: a cubic ease-out to the far corner, then a
/// smoothstep fade at full size. Both curves, and both durations, are the
/// original's.
fn state(circle: &Circle, now_ms: u32) -> (i32, u8) {
    let age_ms = now_ms.wrapping_sub(circle.born_ms);
    if age_ms < GROW_MS {
        let eased = smoothstep_q15(age_ms, GROW_MS);
        return ((circle.full_r as u32 * eased >> 15) as i32, 255);
    }
    let fade_age = age_ms - GROW_MS;
    if fade_age >= FADE_MS {
        return (circle.full_r, 0);
    }
    let smooth = smoothstep_q15(fade_age, FADE_MS);
    (circle.full_r, (((32_768 - smooth) * 255) >> 15) as u8)
}

/// Q15 throughout, so every product stays inside a u32 and RV32 never reaches
/// for multiword arithmetic.
#[inline]
fn smoothstep_q15(elapsed: u32, duration: u32) -> u32 {
    let t = elapsed * 32_768 / duration;
    let squared = t * t >> 15;
    squared * (3 * 32_768 - 2 * t) >> 15
}
