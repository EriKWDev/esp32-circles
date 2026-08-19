//! Two-player pong, and an excuse to fire the circles demo from something other
//! than a fingertip.
//!
//! Every bounce spawns one of `bubbles`' expanding circles: small and neutral off
//! a wall, larger and in the bat's own colour off a bat, and a screen-filling one
//! in the winner's colour when a point lands. The game itself draws almost
//! nothing - two bars, a ball and the score - so the circles are the spectacle.
//!
//! Both players sit at the same panel: drag anywhere on your half to move your
//! bat. The touch controller reports one contact at a time, so two people cannot
//! actually move at once - which turns out to play fine, since the ball only
//! threatens one end at a time.

use crate::bubbles::Bubbles;
use crate::gfx::{Align, H, Scene, W, rgb};
use crate::font::FontId;

pub const LEFT_COLOR: u16 = rgb(70, 150, 255);
pub const RIGHT_COLOR: u16 = rgb(255, 150, 40);
const BALL_COLOR: u16 = rgb(240, 246, 250);

const BAT_X: i32 = 26;
const BAT_W: i32 = 12;
const BAT_H: i32 = 96;
const BALL_R: i32 = 9;
/// Sub-pixel positions, so a shallow angle does not degenerate into a staircase.
const Q: i32 = 8;
/// Pixels per second, at Q. Fast enough to be a game, slow enough to return.
const SPEED: i32 = 210 * Q;
/// Added to the ball's speed on every bat hit, up to a ceiling.
const SPEED_GAIN: i32 = 12 * Q;
const SPEED_MAX: i32 = 460 * Q;
/// How long the ball waits at the centre after a point.
const SERVE_PAUSE_MS: u32 = 900;

pub struct Pong {
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
    left_y: i32,
    right_y: i32,
    pub left_score: u8,
    pub right_score: u8,
    serve_at_ms: u32,
    /// Which way to serve next, so the loser is not immediately under pressure
    /// again from the same direction.
    serve_left: bool,
}

impl Pong {
    pub const fn new() -> Self {
        Self {
            x: (W as i32 / 2) * Q,
            y: (H as i32 / 2) * Q,
            vx: 0,
            vy: 0,
            left_y: H as i32 / 2,
            right_y: H as i32 / 2,
            left_score: 0,
            right_score: 0,
            serve_at_ms: 0,
            serve_left: false,
        }
    }

    /// Fresh game, centred bats, ball held for a moment before it moves.
    pub fn reset(&mut self, now_ms: u32) {
        *self = Self::new();
        self.serve_at_ms = now_ms + SERVE_PAUSE_MS;
    }

    /// Drag on a half moves that half's bat. The bat centres on the finger rather
    /// than following it by an offset, which is what makes a fast return possible.
    pub fn touch(&mut self, x: i32, y: i32) {
        let limit = (BAT_H / 2, H as i32 - BAT_H / 2);
        let y = y.clamp(limit.0, limit.1);
        if x < W as i32 / 2 {
            self.left_y = y;
        } else {
            self.right_y = y;
        }
    }

    pub fn update(&mut self, dt_ms: u32, now_ms: u32, bubbles: &mut Bubbles) {
        if now_ms < self.serve_at_ms {
            return;
        }
        if self.vx == 0 {
            // Serving. The vertical component is taken from the clock so the
            // opening is never quite the same twice.
            self.vx = if self.serve_left { -SPEED } else { SPEED };
            self.vy = ((now_ms % 7) as i32 - 3) * (SPEED / 6);
            self.serve_left = !self.serve_left;
        }

        // Cap the step so a long frame - a network transfer, a transition - cannot
        // teleport the ball through a bat.
        let dt = dt_ms.min(40) as i32;
        self.x += self.vx * dt / 1000;
        self.y += self.vy * dt / 1000;

        // Walls.
        let top = BALL_R * Q;
        let bottom = (H as i32 - BALL_R) * Q;
        if self.y <= top && self.vy < 0 || self.y >= bottom && self.vy > 0 {
            self.y = self.y.clamp(top, bottom);
            self.vy = -self.vy;
            bubbles.spawn(self.x / Q, self.y / Q, now_ms, None, Some(70));
        }

        // Bats. Tested as the ball crossing the bat's face while overlapping it
        // vertically, so a fast ball is caught by where it *was* as well as where
        // it is.
        let face_left = (BAT_X + BAT_W + BALL_R) * Q;
        let face_right = (W as i32 - BAT_X - BAT_W - BALL_R) * Q;
        if self.vx < 0 && self.x <= face_left && self.hits(self.left_y) {
            self.bounce_off_bat(face_left, self.left_y, LEFT_COLOR, now_ms, bubbles);
        } else if self.vx > 0 && self.x >= face_right && self.hits(self.right_y) {
            self.bounce_off_bat(face_right, self.right_y, RIGHT_COLOR, now_ms, bubbles);
        }

        // Out. The winner's colour swallows the screen.
        if self.x < -BALL_R * Q {
            self.right_score = self.right_score.saturating_add(1);
            self.point(now_ms, RIGHT_COLOR, bubbles);
        } else if self.x > (W as i32 + BALL_R) * Q {
            self.left_score = self.left_score.saturating_add(1);
            self.point(now_ms, LEFT_COLOR, bubbles);
        }
    }

    fn hits(&self, bat_y: i32) -> bool {
        let reach = (BAT_H / 2 + BALL_R) * Q;
        (self.y - bat_y * Q).abs() <= reach
    }

    fn bounce_off_bat(
        &mut self,
        face: i32,
        bat_y: i32,
        color: u16,
        now_ms: u32,
        bubbles: &mut Bubbles,
    ) {
        self.x = face;
        self.vx = -self.vx;
        // Where on the bat it landed steers the return, which is the whole game.
        let offset = (self.y - bat_y * Q) / Q;
        self.vy += offset * (SPEED / 40);
        let speed = self.vx.abs() + SPEED_GAIN;
        self.vx = self.vx.signum() * speed.min(SPEED_MAX);
        self.vy = self.vy.clamp(-SPEED_MAX, SPEED_MAX);
        bubbles.spawn(self.x / Q, self.y / Q, now_ms, Some(color), Some(150));
    }

    fn point(&mut self, now_ms: u32, color: u16, bubbles: &mut Bubbles) {
        bubbles.spawn(W as i32 / 2, H as i32 / 2, now_ms, Some(color), None);
        self.x = (W as i32 / 2) * Q;
        self.y = (H as i32 / 2) * Q;
        self.vx = 0;
        self.vy = 0;
        self.serve_at_ms = now_ms + SERVE_PAUSE_MS;
    }

    pub fn draw(&self, scene: &mut Scene, alpha: u8) {
        // Centre line, as a dashed column of pills.
        let mut y = 18;
        while y < H as i32 - 18 {
            scene.pill(
                W as i32 / 2 - 2,
                y,
                W as i32 / 2 + 2,
                y + 14,
                2,
                rgb(26, 34, 42),
                alpha,
            );
            y += 28;
        }

        for (x, cy, color) in [
            (BAT_X, self.left_y, LEFT_COLOR),
            (W as i32 - BAT_X - BAT_W, self.right_y, RIGHT_COLOR),
        ] {
            scene.pill(
                x,
                cy - BAT_H / 2,
                x + BAT_W,
                cy + BAT_H / 2,
                BAT_W / 2,
                color,
                alpha,
            );
        }

        scene.disc(self.x / Q, self.y / Q, BALL_R, BALL_COLOR, alpha);

        // Scores sit either side of the centre line, each in its own colour.
        for (x, score, color) in [
            (W as i32 / 2 - 40, self.left_score, LEFT_COLOR),
            (W as i32 / 2 + 40, self.right_score, RIGHT_COLOR),
        ] {
            let tens = score / 10;
            let digits = [b'0' + tens, b'0' + score % 10];
            let shown = if tens > 0 { &digits[..] } else { &digits[1..] };
            let text = crate::gfx::Text::new(core::str::from_utf8(shown).unwrap_or("?"));
            scene.label(
                x,
                84,
                FontId::Display,
                color,
                alpha,
                Align::Center,
                text.as_str(),
            );
        }
    }
}
