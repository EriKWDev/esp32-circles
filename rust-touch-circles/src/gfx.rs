//! Scanline renderer for the irrigation UI.
//!
//! Shapes are evaluated analytically per scanline rather than kept in a
//! framebuffer - there is no RAM for a 480x480x2 = 450 KiB one, and the panel
//! wants pixels streamed in stripes anyway. Each row is composited straight
//! into the DMA stripe buffer that is about to be sent.
//!
//! Two things differ from the circles demo this grew out of. The demo relied on
//! an occlusion interval list, because 32 screen-filling opaque circles overdraw
//! badly; a UI draws a handful of small shapes, so painting back-to-front is
//! cheaper than tracking coverage. And edges are alpha-blended against what is
//! already in the row instead of Bayer-dithered: the blend path has to exist for
//! text regardless, and a genuinely smooth edge is worth the few cycles on the
//! dozen or so boundary pixels per shape per row.

#![allow(clippy::too_many_arguments)] // primitive APIs mirror their geometry directly

use crate::font::FontId;

pub const W: usize = 480;
pub const H: usize = 480;
pub const STRIPE_ROWS: usize = 32;
pub const STRIPE_BYTES: usize = W * STRIPE_ROWS * 2;

// Worst case is the schedule editor: six entry rows of six primitives each, the
// clock adjusters, the six entry controls, and a scrollbar - with ripples and a
// running badge on top. Fixed storage keeps memory use deterministic; the
// headroom exists because anything past the cap is silently dropped, so a shortage
// would present as a button that is simply not drawn. `prims=` in the heartbeat
// reports the peak against this.
pub const MAX_PRIMS: usize = 112;
/// Longest string any single text primitive can hold. Relay names are the only
/// unbounded input and are truncated to this on the way in.
pub const MAX_TEXT: usize = 28;

#[inline(always)]
pub const fn rgb(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0xf8) << 8) | ((g as u16 & 0xfc) << 3) | (b as u16 >> 3)
}

/// The same hue at roughly half strength, for decoration that must not compete
/// with what it sits behind.
pub const fn muted(color: u16) -> u16 {
    let r = ((color >> 11) & 0x1f) * 9 / 16;
    let g = ((color >> 5) & 0x3f) * 9 / 16;
    let b = (color & 0x1f) * 9 / 16;
    (r << 11) | (g << 5) | b
}

/// Blend `src` over `dst` by `cov` (0..=255), in RGB565.
///
/// Kept branch-light and integer-only: RV32IMAC has no FPU, and this runs on
/// every anti-aliased edge pixel and every soft glyph pixel on screen.
#[inline(always)]
fn blend(dst: u16, src: u16, cov: u8) -> u16 {
    if cov == 0 {
        return dst;
    }
    if cov == 255 {
        return src;
    }
    let a = cov as u32 + 1;
    let ia = 256 - cov as u32;
    // Channels stay in their packed lanes; no unpack to 8 bits and back.
    let dr = (dst >> 11) as u32 & 0x1f;
    let dg = (dst >> 5) as u32 & 0x3f;
    let db = dst as u32 & 0x1f;
    let sr = (src >> 11) as u32 & 0x1f;
    let sg = (src >> 5) as u32 & 0x3f;
    let sb = src as u32 & 0x1f;
    let r = (sr * a + dr * ia) >> 8;
    let g = (sg * a + dg * ia) >> 8;
    let b = (sb * a + db * ia) >> 8;
    ((r as u16) << 11) | ((g as u16) << 5) | b as u16
}

// The single-pixel accessors bounds-check against the row they were handed
// rather than against W. Callers already clamp, so this never fires in normal
// operation; it exists so that a future geometry mistake costs one missing pixel
// instead of a panic on a wall-mounted panel or a corrupted DMA buffer.
#[inline(always)]
fn put(row: &mut [u8], x: usize, color: u16) {
    let byte = x * 2;
    if byte + 1 >= row.len() {
        return;
    }
    row[byte] = (color >> 8) as u8;
    row[byte + 1] = color as u8;
}

#[inline(always)]
fn get(row: &[u8], x: usize) -> u16 {
    let byte = x * 2;
    if byte + 1 >= row.len() {
        return 0;
    }
    ((row[byte] as u16) << 8) | row[byte + 1] as u16
}

#[inline(always)]
fn blend_at(row: &mut [u8], x: usize, color: u16, cov: u8) {
    match cov {
        0 => {}
        255 => put(row, x, color),
        _ => {
            let dst = get(row, x);
            put(row, x, blend(dst, color, cov));
        }
    }
}

/// Opaque horizontal run, two pixels per 32-bit store where alignment allows.
/// Lifted from the circles demo - it is the single hottest loop in the renderer.
#[inline]
fn fill_span(row: &mut [u8], left: i32, right: i32, color: u16) {
    // Clamp entirely in signed space, and only then convert.
    //
    // This used to cast before comparing, which was badly wrong for a span that
    // lies off-screen to the left: `right` is negative there, the `right < left`
    // test passes because both sides are negative, and `-20 as usize` becomes
    // about four billion - so the loop below walked straight off the end of the
    // DMA buffer. In `fill_span` that corrupted memory silently through the
    // unchecked stores; in `blend_span` it panicked. Shapes drift off-screen
    // constantly here (every ripple and every transition disc), so this was
    // reachable in normal use.
    let left = left.max(0);
    let right = right.min(W as i32 - 1);
    if right < left {
        return;
    }
    let left = left as usize;
    let right = right as usize;
    let hi = (color >> 8) as u8;
    let lo = color as u8;
    let pair = u32::from_le_bytes([hi, lo, hi, lo]);
    let mut x = left;
    if x & 1 != 0 {
        put(row, x, color);
        x += 1;
    }
    // The paired store is the renderer's hottest instruction, so it stays
    // unchecked - but it is bounded by `limit` derived from the row's own length,
    // not from W, so a wrong W or a short slice cannot turn it into a stray write.
    let limit = row.len() / 2;
    while x < right && x + 1 < limit {
        // x is even here, so row + x*2 is 4-byte aligned within the stripe.
        unsafe { (row.as_mut_ptr().add(x * 2) as *mut u32).write(pair) };
        x += 2;
    }
    if x <= right {
        put(row, x, color);
    }
}

/// A blended horizontal run - used when a shape carries a fade alpha.
#[inline]
fn blend_span(row: &mut [u8], left: i32, right: i32, color: u16, cov: u8) {
    if cov == 255 {
        fill_span(row, left, right, color);
        return;
    }
    if cov == 0 {
        return;
    }
    // Signed clamp before conversion - see fill_span.
    let left = left.max(0);
    let right = right.min(W as i32 - 1);
    if right < left {
        return;
    }
    for x in left as usize..=right as usize {
        blend_at(row, x, color, cov);
    }
}

#[inline]
fn isqrt(n: u32) -> u32 {
    n.isqrt()
}

#[derive(Clone, Copy)]
pub struct Text {
    bytes: [u8; MAX_TEXT],
    pub len: u8,
}

impl Text {
    pub const EMPTY: Text = Text {
        bytes: [0; MAX_TEXT],
        len: 0,
    };

    pub fn new(s: &str) -> Self {
        let mut t = Text::EMPTY;
        // Truncate on a char boundary so a clipped name never becomes invalid
        // UTF-8 (relay names contain å/ä/ö, which are two bytes each).
        let take = s
            .char_indices()
            .map(|(i, c)| i + c.len_utf8())
            .take_while(|&e| e <= MAX_TEXT)
            .last()
            .unwrap_or(0);
        t.bytes[..take].copy_from_slice(&s.as_bytes()[..take]);
        t.len = take as u8;
        t
    }

    pub fn as_str(&self) -> &str {
        // SAFETY: `Text::new` copies only through a UTF-8 char boundary and
        // `EMPTY` has length zero. The bytes are private and therefore can only
        // be produced by those constructors; avoiding validation here matters because
        // the scanline renderer asks for this slice on every glyph row.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len as usize]) }
    }
}

/// A scratch buffer for formatting a label, capped at what a `Text` can hold.
/// Overflow is dropped rather than panicking - a clipped caption is better than a
/// dead panel.
pub struct TextBuf {
    bytes: [u8; MAX_TEXT],
    len: usize,
}

impl TextBuf {
    pub const fn new() -> Self {
        Self {
            bytes: [0; MAX_TEXT],
            len: 0,
        }
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

impl core::fmt::Write for TextBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            if self.len < MAX_TEXT {
                self.bytes[self.len] = b;
                self.len += 1;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy)]
pub enum Prim {
    /// Filled circle. Doubles as the screen-covering transition wipe.
    Disc {
        cx: i32,
        cy: i32,
        r: i32,
        color: u16,
        alpha: u8,
    },
    /// Annulus - selection rings and the countdown progress track.
    Ring {
        cx: i32,
        cy: i32,
        r_outer: i32,
        r_inner: i32,
        color: u16,
        alpha: u8,
    },
    /// Rounded rectangle / pill. `r` is the corner radius.
    Pill {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        r: i32,
        color: u16,
        alpha: u8,
    },
    /// A circular-arc wedge of a ring, for the countdown progress sweep.
    /// `from`/`to` are in turns, Q12 (0..4096 == full circle), measured
    /// clockwise from 12 o'clock.
    Arc {
        cx: i32,
        cy: i32,
        r_outer: i32,
        r_inner: i32,
        from: i32,
        to: i32,
        color: u16,
        alpha: u8,
    },
    Label {
        /// Left edge of the laid-out text. Alignment is resolved once when the
        /// scene is built, never redundantly on each covered scanline.
        x: i32,
        baseline: i32,
        font: FontId,
        color: u16,
        alpha: u8,
        text: Text,
    },
    /// One keyboard row: evenly spaced plates, one character on each.
    ///
    /// Exists because two primitives per key is about eighty for QWERTY, past
    /// MAX_PRIMS on its own. A row is a regular grid, so it collapses into one.
    KeyRow {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        /// Corner radius of each key plate.
        r: i32,
        /// Horizontal gap between adjacent plates.
        gap: i32,
        plate: u16,
        /// Plate colour for the key at `highlight`.
        hot: u16,
        ink: u16,
        alpha: u8,
        /// Slot drawn with `hot`, or -1 for none.
        highlight: i8,
        font: FontId,
        /// One character per slot; its length sets the slot count.
        text: Text,
    },
}

/// How many discs the occluding batch below can hold. Matches the demo's circle
/// limit, since that is the only thing that fills it.
pub const MAX_DISCS: usize = 32;

/// One disc of the occluding batch.
#[derive(Clone, Copy)]
pub struct Disc {
    pub cx: i32,
    pub cy: i32,
    pub r: i32,
    pub color: u16,
    pub alpha: u8,
}

const NO_DISC: Disc = Disc {
    cx: 0,
    cy: 0,
    r: 0,
    color: 0,
    alpha: 0,
};

pub struct Scene {
    pub prims: [Prim; MAX_PRIMS],
    /// Precomputed inclusive vertical bounds for each primitive. This removes
    /// geometry/font dispatch from the scanline hot loop.
    rows: [(i16, i16); MAX_PRIMS],
    /// Current build-time vertical clip. It is folded into `rows` when a
    /// primitive is appended, so scanline rendering has no clip state to
    /// interpret in its hot loop.
    clip_top: i16,
    clip_bottom: i16,
    pub len: usize,
    pub background: u16,
    /// Discs composited with occlusion, beneath all primitives.
    ///
    /// A separate path because the assumption behind the primitive list - a
    /// handful of small shapes, so back-to-front painting beats tracking coverage
    /// - is exactly wrong for the circles demo, where a dozen overlapping
    /// screen-filling discs can each cost a full-screen fill. `discs_row` writes
    /// every pixel once instead, which is what let the original demo hold 60 fps.
    pub discs: [Disc; MAX_DISCS],
    pub n_discs: usize,
}

const NOTHING: Prim = Prim::Disc {
    cx: 0,
    cy: 0,
    r: 0,
    color: 0,
    alpha: 0,
};

impl Scene {
    pub fn new() -> Self {
        Self {
            prims: [NOTHING; MAX_PRIMS],
            rows: [(0, -1); MAX_PRIMS],
            clip_top: 0,
            clip_bottom: H as i16 - 1,
            len: 0,
            background: 0,
            discs: [NO_DISC; MAX_DISCS],
            n_discs: 0,
        }
    }

    pub fn clear(&mut self, background: u16) {
        self.len = 0;
        self.n_discs = 0;
        self.background = background;
        self.clip_top = 0;
        self.clip_bottom = H as i16 - 1;
    }

    /// Add a disc to the occluding batch. Order matters: later discs are on top.
    pub fn push_disc(&mut self, cx: i32, cy: i32, r: i32, color: u16, alpha: u8) {
        if self.n_discs < MAX_DISCS {
            self.discs[self.n_discs] = Disc {
                cx,
                cy,
                r,
                color,
                alpha,
            };
            self.n_discs += 1;
        }
    }

    #[inline]
    pub fn push(&mut self, p: Prim) {
        let (top, bottom) = prim_rows(&p);
        self.push_rows(p, top, bottom);
    }

    #[inline]
    fn push_rows(&mut self, p: Prim, top: i32, bottom: i32) {
        debug_assert!(self.len < MAX_PRIMS, "scene primitive capacity exceeded");
        if self.len < MAX_PRIMS {
            self.prims[self.len] = p;
            self.rows[self.len] = (
                (top as i16).max(self.clip_top),
                (bottom as i16).min(self.clip_bottom),
            );
            self.len += 1;
        }
    }

    pub fn disc(&mut self, cx: i32, cy: i32, r: i32, color: u16, alpha: u8) {
        self.push(Prim::Disc {
            cx,
            cy,
            r,
            color,
            alpha,
        });
    }

    pub fn ring(&mut self, cx: i32, cy: i32, r_outer: i32, r_inner: i32, color: u16, alpha: u8) {
        self.push(Prim::Ring {
            cx,
            cy,
            r_outer,
            r_inner,
            color,
            alpha,
        });
    }

    pub fn pill(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, r: i32, color: u16, alpha: u8) {
        self.push(Prim::Pill {
            x0,
            y0,
            x1,
            y1,
            r,
            color,
            alpha,
        });
    }

    /// One keyboard row. `text` supplies the key captions, one character each.
    pub fn key_row(
        &mut self,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        r: i32,
        gap: i32,
        plate: u16,
        hot: u16,
        ink: u16,
        alpha: u8,
        highlight: i8,
        font: FontId,
        text: &str,
    ) {
        self.push(Prim::KeyRow {
            x0,
            y0,
            x1,
            y1,
            r,
            gap,
            plate,
            hot,
            ink,
            alpha,
            highlight,
            font,
            text: Text::new(text),
        });
    }

    pub fn clip(&mut self, y0: i32, y1: i32) {
        self.clip_top = y0.clamp(0, H as i32 - 1) as i16;
        self.clip_bottom = y1.clamp(-1, H as i32 - 1) as i16;
    }

    pub fn clip_reset(&mut self) {
        self.clip_top = 0;
        self.clip_bottom = H as i16 - 1;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn arc(
        &mut self,
        cx: i32,
        cy: i32,
        r_outer: i32,
        r_inner: i32,
        from: i32,
        to: i32,
        color: u16,
        alpha: u8,
    ) {
        self.push(Prim::Arc {
            cx,
            cy,
            r_outer,
            r_inner,
            from,
            to,
            color,
            alpha,
        });
    }

    pub fn label(
        &mut self,
        x: i32,
        baseline: i32,
        font: FontId,
        color: u16,
        alpha: u8,
        align: Align,
        text: &str,
    ) {
        let text = Text::new(text);
        let x = match align {
            Align::Left => x,
            Align::Center => x - font.get().width(text.as_str()) / 2,
            Align::Right => x - font.get().width(text.as_str()),
        };
        let f = font.get();
        let mut top = baseline;
        let mut bottom = baseline;
        for ch in text.as_str().chars() {
            if let Some(g) = f.glyph(ch) {
                top = top.min(baseline + g.top - 1);
                bottom = bottom.max(baseline + g.top + g.h as i32 + 1);
            }
        }
        self.push_rows(
            Prim::Label {
                x,
                baseline,
                font,
                color,
                alpha,
                text,
            },
            top,
            bottom,
        );
    }
}

/// Bounds of key slot `index` of `n`. The only place this arithmetic exists: the
/// renderer places plates with it and the UI resolves touches with it, so a key
/// cannot drift from the region that presses it.
pub fn key_slot(x0: i32, x1: i32, gap: i32, n: usize, index: usize) -> (i32, i32) {
    let span = x1 - x0;
    let n = n.max(1) as i32;
    let left = x0 + span * index as i32 / n;
    let right = x0 + span * (index as i32 + 1) / n;
    (left + gap / 2, right - gap / 2)
}

/// Which key of `n` a touch at `x` falls on, or None outside the band.
pub fn key_slot_at(x0: i32, x1: i32, n: usize, x: i32) -> Option<usize> {
    if n == 0 || x < x0 || x > x1 {
        return None;
    }
    // Gaps deliberately do not reject: a finger landing between two plates
    // should still press the nearer key rather than nothing at all.
    Some((((x - x0) * n as i32 / (x1 - x0).max(1)) as usize).min(n - 1))
}

/// Vertical bounds of a primitive, so a stripe can skip primitives entirely.
fn prim_rows(p: &Prim) -> (i32, i32) {
    match *p {
        Prim::Disc { cy, r, .. } => (cy - r - 1, cy + r + 1),
        Prim::Ring { cy, r_outer, .. } | Prim::Arc { cy, r_outer, .. } => {
            (cy - r_outer - 1, cy + r_outer + 1)
        }
        Prim::Pill { y0, y1, .. } => (y0 - 1, y1 + 1),
        Prim::Label { baseline, font, .. } => {
            let f = font.get();
            (baseline - f.ascent - 2, baseline + f.px + 2)
        }
        Prim::KeyRow { y0, y1, .. } => (y0 - 1, y1 + 1),
    }
}

/// Horizontal half-extent of a circle of radius `r` at vertical distance `dy`,
/// in Q4 - the shared core of disc, ring and arc scanline evaluation.
#[inline(always)]
fn half_extent_q(r_q: i32, dy_q: i32) -> i32 {
    let rr = (r_q * r_q) as u32;
    let dd = (dy_q * dy_q) as u32;
    if dd >= rr {
        return -1;
    }
    isqrt(rr - dd) as i32
}

/// Paint one horizontal slice of a filled circle, anti-aliasing the two edge
/// pixels by sub-pixel coverage.
#[inline]
fn circle_row(row: &mut [u8], cx: i32, extent_q: i32, color: u16, alpha: u8) {
    if extent_q < 0 {
        return;
    }
    let left_q = (cx << 4) - extent_q;
    let right_q = (cx << 4) + extent_q;
    // Interior: whole pixels fully inside the shape.
    let inner_left = (left_q + 15) >> 4;
    let inner_right = (right_q - 15) >> 4;
    blend_span(row, inner_left, inner_right, color, alpha);
    // Edges: coverage from how much of the pixel the shape actually spans.
    let le = inner_left - 1;
    if le >= 0 && le < W as i32 {
        let cov = (((le + 1) << 4) - left_q).clamp(0, 16) as u32;
        let cov = (cov * alpha as u32 / 16) as u8;
        blend_at(row, le as usize, color, cov);
    }
    let re = inner_right + 1;
    if re >= 0 && re < W as i32 && re != le {
        let cov = (right_q - (re << 4)).clamp(0, 16) as u32;
        let cov = (cov * alpha as u32 / 16) as u8;
        blend_at(row, re as usize, color, cov);
    }
}

/// Two slices, left and right of the hole, for a ring at this row.
#[inline]
fn ring_row(row: &mut [u8], cx: i32, outer_q: i32, inner_q: i32, color: u16, alpha: u8) {
    if outer_q < 0 {
        return;
    }
    if inner_q < 0 {
        circle_row(row, cx, outer_q, color, alpha);
        return;
    }
    let ol = (cx << 4) - outer_q;
    let or = (cx << 4) + outer_q;
    let il = (cx << 4) - inner_q;
    let ir = (cx << 4) + inner_q;

    // Outer edge in, up to the hole.
    let seg_l0 = (ol + 15) >> 4;
    let seg_l1 = (il - 15) >> 4;
    blend_span(row, seg_l0, seg_l1, color, alpha);
    let seg_r0 = (ir + 15) >> 4;
    let seg_r1 = (or - 15) >> 4;
    blend_span(row, seg_r0, seg_r1, color, alpha);

    // Four anti-aliased boundaries: outer-left, inner-left, inner-right,
    // outer-right.
    let edges = [
        (seg_l0 - 1, (((seg_l0) << 4) - ol).clamp(0, 16)),
        (seg_l1 + 1, (il - ((seg_l1 + 1) << 4)).clamp(0, 16)),
        (seg_r0 - 1, (((seg_r0) << 4) - ir).clamp(0, 16)),
        (seg_r1 + 1, (or - ((seg_r1 + 1) << 4)).clamp(0, 16)),
    ];
    for (x, cov) in edges {
        if x >= 0 && x < W as i32 {
            let cov = (cov as u32 * alpha as u32 / 16) as u8;
            blend_at(row, x as usize, color, cov);
        }
    }
}

/// Quarter-wave sine, Q12 (4096 == 1.0), indexed by turn/16 over 0..=1024.
/// 65 entries with linear interpolation is accurate to about 0.05% - far finer
/// than a pixel at this panel's radii.
static SIN_Q12: [i16; 65] = [
    0, 100, 201, 301, 401, 501, 601, 700, 799, 897, 995, 1092, 1189, 1285, 1380, 1474, 1567, 1660,
    1751, 1842, 1931, 2019, 2106, 2191, 2276, 2359, 2440, 2520, 2598, 2675, 2751, 2824, 2896, 2967,
    3035, 3102, 3166, 3229, 3290, 3349, 3406, 3461, 3513, 3564, 3612, 3659, 3703, 3745, 3784, 3822,
    3857, 3889, 3920, 3948, 3973, 3996, 4017, 4036, 4052, 4065, 4076, 4085, 4091, 4095, 4096,
];

/// sin of a Q12 turn (0..4096 == full circle), returned in Q12.
#[inline]
fn sin_q12(turn: i32) -> i32 {
    let t = turn & 4095;
    let (quadrant, within) = (t >> 10, t & 1023);
    let idx = (within >> 4) as usize;
    let frac = within & 15;
    let lerp = |i: usize| -> i32 {
        let a = SIN_Q12[i] as i32;
        let b = SIN_Q12[(i + 1).min(64)] as i32;
        a + (b - a) * frac / 16
    };
    match quadrant {
        0 => lerp(idx),
        1 => lerp(64 - idx - if frac > 0 { 1 } else { 0 }).max(0),
        2 => -lerp(idx),
        _ => -lerp(64 - idx - if frac > 0 { 1 } else { 0 }).max(0),
    }
}

#[inline]
fn cos_q12(turn: i32) -> i32 {
    sin_q12(turn + 1024)
}

/// Unit direction of a clockwise turn from 12 o'clock, in Q12 screen coords
/// (y grows downward, so 12 o'clock is (0, -1)).
#[inline]
fn dir_q12(turn: i32) -> (i32, i32) {
    (sin_q12(turn), -cos_q12(turn))
}

/// Is (dx, dy) inside the clockwise sector from `a` to `b`?
///
/// Pure cross products, so there is not a single division in the inner loop.
/// The previous version derived each pixel's angle with an integer arctangent -
/// two divisions per pixel, over roughly 17k pixels for a ring this size, which
/// cost milliseconds per frame and was exactly why the sweep looked stuttery.
#[inline(always)]
fn in_sector(dx: i32, dy: i32, a: (i32, i32), b: (i32, i32), span_over_half: bool) -> bool {
    // cross(u, p) > 0 means p is clockwise of u, in screen coordinates.
    let ca = a.0 * dy - a.1 * dx;
    let cb = b.0 * dy - b.1 * dx;
    if span_over_half {
        ca >= 0 || cb <= 0
    } else {
        ca >= 0 && cb <= 0
    }
}

#[inline]
fn arc_row(
    row: &mut [u8],
    cx: i32,
    cy: i32,
    y: i32,
    r_outer: i32,
    r_inner: i32,
    from: i32,
    to: i32,
    color: u16,
    alpha: u8,
) {
    let dy = y - cy;
    let outer_q = half_extent_q(r_outer << 4, dy << 4);
    if outer_q < 0 {
        return;
    }
    let inner_q = half_extent_q(r_inner << 4, dy << 4);
    let a = dir_q12(from);
    let b = dir_q12(to);
    let span_over_half = ((to - from) & 4095) > 2048;
    let outer2 = (r_outer * r_outer) as u32;
    let inner2 = (r_inner * r_inner) as u32;

    // Walk only the two bands the annulus actually occupies on this row, so the
    // hole in the middle costs nothing.
    let outer_left = ((cx << 4) - outer_q) >> 4;
    let outer_right = ((cx << 4) + outer_q) >> 4;
    let mut bands: [(i32, i32); 2] = [(outer_left, outer_right), (0, -1)];
    if inner_q >= 0 {
        let inner_left = ((cx << 4) - inner_q) >> 4;
        let inner_right = ((cx << 4) + inner_q) >> 4;
        bands = [(outer_left, inner_left), (inner_right, outer_right)];
    }

    for (lo, hi) in bands {
        if hi < lo {
            continue;
        }
        for x in lo.max(0)..=hi.min(W as i32 - 1) {
            let dx = x - cx;
            let dist2 = (dx * dx + dy * dy) as u32;
            if dist2 > outer2 || dist2 < inner2 {
                continue;
            }
            if in_sector(dx, dy, a, b, span_over_half) {
                blend_at(row, x as usize, color, alpha);
            }
        }
    }
}

#[inline]
fn pill_row(
    row: &mut [u8],
    y: i32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    r: i32,
    color: u16,
    alpha: u8,
) {
    if y < y0 || y > y1 {
        return;
    }
    // Straight middle section: full width.
    if y >= y0 + r && y <= y1 - r {
        blend_span(row, x0, x1, color, alpha);
        return;
    }
    // Corner rows: inset by the arc, anti-aliased at both ends.
    let cy = if y < y0 + r { y0 + r } else { y1 - r };
    let dy = (y - cy).abs();
    let extent_q = half_extent_q(r << 4, dy << 4);
    if extent_q < 0 {
        return;
    }
    let inset_q = (r << 4) - extent_q;
    let left_q = (x0 << 4) + inset_q;
    let right_q = (x1 << 4) - inset_q;
    let inner_left = (left_q + 15) >> 4;
    let inner_right = (right_q - 15) >> 4;
    blend_span(row, inner_left, inner_right, color, alpha);
    let le = inner_left - 1;
    if le >= 0 && le < W as i32 {
        let cov = (((le + 1) << 4) - left_q).clamp(0, 16) as u32;
        blend_at(row, le as usize, color, (cov * alpha as u32 / 16) as u8);
    }
    let re = inner_right + 1;
    if re >= 0 && re < W as i32 && re != le {
        let cov = (right_q - (re << 4)).clamp(0, 16) as u32;
        blend_at(row, re as usize, color, (cov * alpha as u32 / 16) as u8);
    }
}

#[inline]
fn label_row(
    row: &mut [u8],
    y: i32,
    x: i32,
    baseline: i32,
    font: FontId,
    color: u16,
    alpha: u8,
    text: &str,
) {
    let mut pen = x;
    for ch in text.chars() {
        pen += glyph_row(row, y, pen, baseline, font, color, alpha, ch);
    }
}

/// One glyph's contribution to this scanline. Returns the pen advance, so the
/// caller decides whether characters are laid out in sequence (`label_row`) or
/// placed individually (`key_row_prim`).
#[inline]
fn glyph_row(
    row: &mut [u8],
    y: i32,
    pen: i32,
    baseline: i32,
    font: FontId,
    color: u16,
    alpha: u8,
    ch: char,
) -> i32 {
    let f = font.get();
    let Some(g) = f.glyph(ch) else {
        return f.px / 3;
    };
    let gy = y - (baseline + g.top);
    if gy >= 0 && gy < g.h as i32 {
        let coverage = f.row(g, gy as usize);
        let gx0 = pen + g.left;
        for (i, &cov) in coverage.iter().enumerate() {
            if cov == 0 {
                continue;
            }
            let px = gx0 + i as i32;
            if px < 0 || px >= W as i32 {
                continue;
            }
            let cov = if alpha == 255 {
                cov
            } else {
                ((cov as u32 * alpha as u32) / 255) as u8
            };
            blend_at(row, px as usize, color, cov);
        }
    }
    g.advance
}

/// One scanline of a keyboard row: every plate that this row crosses, then the
/// captions on top of them.
#[inline]
fn key_row_prim(
    row: &mut [u8],
    y: i32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    r: i32,
    gap: i32,
    plate: u16,
    hot: u16,
    ink: u16,
    alpha: u8,
    highlight: i8,
    font: FontId,
    text: &str,
) {
    let n = text.chars().count();
    if n == 0 {
        return;
    }
    let f = font.get();
    // Captions sit on a baseline that centres the cap height in the plate,
    // computed from the font's own ascent rather than a tuned constant.
    let baseline = (y0 + y1) / 2 + f.ascent / 2 - f.ascent / 8;
    for (index, ch) in text.chars().enumerate() {
        let (kx0, kx1) = key_slot(x0, x1, gap, n, index);
        let color = if index as i8 == highlight { hot } else { plate };
        pill_row(row, y, kx0, y0, kx1, y1, r, color, alpha);
        let advance = f.glyph(ch).map_or(0, |g| g.advance);
        glyph_row(
            row,
            y,
            (kx0 + kx1) / 2 - advance / 2,
            baseline,
            font,
            ink,
            alpha,
            ch,
        );
    }
}

/// One scanline of the occluding disc batch: front-to-back, keeping the intervals
/// already written, so total writes per row are bounded by the row width however
/// many discs overlap. Painting them back-to-front costs a full-screen fill each.
///
/// Edges are hard rather than anti-aliased, as in the demo: a partly transparent
/// edge pixel cannot be treated as covering without leaving seams.
fn discs_row(row: &mut [u8], y: i32, discs: &[Disc]) {
    // Opaque intervals already written, sorted and disjoint.
    let mut covered = [(0i32, 0i32); MAX_DISCS];
    let mut n_covered = 0usize;

    // The opaque discs, nearest first, each filling only what nothing in front of
    // it has claimed. This is where the saving is: without it, every one of them
    // costs close to a full-screen fill.
    for disc in discs.iter().rev().filter(|d| d.alpha == 255) {
        let Some((left, right)) = disc_span(disc, y) else {
            continue;
        };
        paint_uncovered(row, &covered[..n_covered], left, right, disc.color, 255);
        insert_covered(&mut covered, &mut n_covered, left, right);
    }

    // Then the fading ones. A fading disc is always an older one - it has had
    // time to finish growing - so every one of them is *behind* every opaque disc,
    // which is what makes this split sound: they can be blended afterwards as long
    // as they keep out of the opaque spans. Oldest first, so where two of them do
    // overlap they blend in the right order.
    //
    // Skipping the opaque spans is not just tidiness. A fading disc is at full
    // radius, so blending it across the whole row costs far more than filling it;
    // while a drag is piling up new circles, the opaque ones on top mean there is
    // almost nothing of it left to blend.
    for disc in discs.iter().filter(|d| d.alpha != 255) {
        let Some((left, right)) = disc_span(disc, y) else {
            continue;
        };
        paint_uncovered(
            row,
            &covered[..n_covered],
            left,
            right,
            disc.color,
            disc.alpha,
        );
    }
}

/// The x range a disc occupies on scanline `y`, or None if it misses the row.
#[inline]
fn disc_span(disc: &Disc, y: i32) -> Option<(i32, i32)> {
    let dy = (y - disc.cy).abs();
    if dy > disc.r {
        return None;
    }
    let half = ((disc.r * disc.r - dy * dy) as u32).isqrt() as i32;
    Some((disc.cx - half, disc.cx + half))
}

/// Write the parts of `[left, right]` that `covered` does not already own.
/// `blend_span` takes the paired-store fill path at alpha 255, so this serves the
/// opaque and the fading passes alike.
#[inline]
fn paint_uncovered(
    row: &mut [u8],
    covered: &[(i32, i32)],
    left: i32,
    right: i32,
    color: u16,
    alpha: u8,
) {
    let mut cursor = left;
    for &(c0, c1) in covered {
        if c1 < cursor {
            continue;
        }
        if c0 > right {
            break;
        }
        if c0 > cursor {
            blend_span(row, cursor, (c0 - 1).min(right), color, alpha);
        }
        cursor = cursor.max(c1 + 1);
        if cursor > right {
            return;
        }
    }
    blend_span(row, cursor, right, color, alpha);
}

/// Insert `[left, right]` into a sorted, disjoint interval list, coalescing
/// anything it touches. A full list simply stops absorbing new intervals, which
/// costs overdraw but never correctness.
fn insert_covered(covered: &mut [(i32, i32); MAX_DISCS], n: &mut usize, left: i32, right: i32) {
    let mut at = 0;
    while at < *n && covered[at].1 + 1 < left {
        at += 1;
    }
    let mut end = at;
    let (mut low, mut high) = (left, right);
    while end < *n && covered[end].0 <= right + 1 {
        low = low.min(covered[end].0);
        high = high.max(covered[end].1);
        end += 1;
    }
    // Replace the merged run [at, end) with the single interval.
    let merged = end - at;
    if merged == 0 {
        if *n >= MAX_DISCS {
            return;
        }
        covered.copy_within(at..*n, at + 1);
        *n += 1;
    } else if merged > 1 {
        covered.copy_within(end..*n, at + 1);
        *n -= merged - 1;
    }
    covered[at] = (low, high);
}

/// Composite `y0..y0+STRIPE_ROWS` of the scene into `pixels`.
pub fn render_stripe(scene: &Scene, y0: usize, pixels: &mut [u8]) {
    for local in 0..STRIPE_ROWS {
        let y = (y0 + local) as i32;
        let row = &mut pixels[local * W * 2..(local + 1) * W * 2];
        fill_span(row, 0, W as i32 - 1, scene.background);

        // Beneath every primitive, so the Back button and any transition disc are
        // drawn over the demo rather than under it.
        if scene.n_discs != 0 {
            discs_row(row, y, &scene.discs[..scene.n_discs]);
        }

        for index in 0..scene.len {
            let p = &scene.prims[index];
            let (top, bottom) = scene.rows[index];
            let (top, bottom) = (top as i32, bottom as i32);
            if y < top || y > bottom {
                continue;
            }
            match *p {
                Prim::Disc {
                    cx,
                    cy,
                    r,
                    color,
                    alpha,
                } => {
                    let e = half_extent_q(r << 4, (y - cy) << 4);
                    circle_row(row, cx, e, color, alpha);
                }
                Prim::Ring {
                    cx,
                    cy,
                    r_outer,
                    r_inner,
                    color,
                    alpha,
                } => {
                    let dy = (y - cy) << 4;
                    let o = half_extent_q(r_outer << 4, dy);
                    let i = half_extent_q(r_inner << 4, dy);
                    ring_row(row, cx, o, i, color, alpha);
                }
                Prim::Arc {
                    cx,
                    cy,
                    r_outer,
                    r_inner,
                    from,
                    to,
                    color,
                    alpha,
                } => {
                    arc_row(row, cx, cy, y, r_outer, r_inner, from, to, color, alpha);
                }
                Prim::Pill {
                    x0,
                    y0: py0,
                    x1,
                    y1,
                    r,
                    color,
                    alpha,
                } => {
                    pill_row(row, y, x0, py0, x1, y1, r, color, alpha);
                }
                Prim::Label {
                    x,
                    baseline,
                    font,
                    color,
                    alpha,
                    ref text,
                } => {
                    label_row(row, y, x, baseline, font, color, alpha, text.as_str());
                }
                Prim::KeyRow {
                    x0,
                    y0: ky0,
                    x1,
                    y1,
                    r,
                    gap,
                    plate,
                    hot,
                    ink,
                    alpha,
                    highlight,
                    font,
                    ref text,
                } => {
                    key_row_prim(
                        row,
                        y,
                        x0,
                        ky0,
                        x1,
                        y1,
                        r,
                        gap,
                        plate,
                        hot,
                        ink,
                        alpha,
                        highlight,
                        font,
                        text.as_str(),
                    );
                }
            }
        }
    }
}
