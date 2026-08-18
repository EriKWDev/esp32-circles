//! Baked anti-aliased fonts.
//!
//! The coverage maps and metrics are produced by `build.rs` from the bundled
//! Barlow TTFs, at exactly the pixel sizes the UI draws - so nothing is scaled
//! at runtime and glyph edges keep the rasterizer's full 8-bit coverage.

#[derive(Clone, Copy)]
pub struct Glyph {
    pub ch: char,
    pub w: u16,
    pub h: u16,
    /// Horizontal bearing: pen x + `left` is the bitmap's left edge.
    pub left: i32,
    /// Vertical bearing, downward: baseline y + `top` is the bitmap's top row.
    pub top: i32,
    pub advance: i32,
    /// Byte offset of this glyph's coverage rows within `Font::coverage`.
    pub offset: usize,
}

pub struct Font {
    /// Sorted by `ch`, so lookup is a binary search.
    pub glyphs: &'static [Glyph],
    pub coverage: &'static [u8],
    pub ascent: i32,
    pub px: i32,
}

impl Font {
    pub fn glyph(&self, ch: char) -> Option<&'static Glyph> {
        let index = self.glyphs.binary_search_by(|g| g.ch.cmp(&ch)).ok()?;
        Some(&self.glyphs[index])
    }

    /// One glyph's coverage row, `w` bytes of 0..=255 alpha.
    #[inline]
    pub fn row(&self, glyph: &Glyph, y: usize) -> &[u8] {
        let start = glyph.offset + y * glyph.w as usize;
        &self.coverage[start..start + glyph.w as usize]
    }

    /// Advance width of `text`, for centering and right-alignment. Unknown
    /// characters contribute a space so layout never collapses on unexpected
    /// input (relay names are arbitrary text from the controller's config).
    pub fn width(&self, text: &str) -> i32 {
        let mut total = 0;
        for ch in text.chars() {
            total += match self.glyph(ch) {
                Some(g) => g.advance,
                None => self.glyph(' ').map_or(self.px / 3, |g| g.advance),
            };
        }
        total
    }
}

include!(concat!(env!("OUT_DIR"), "/fonts.rs"));

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FontId {
    Icon,
    Micro,
    Countdown,
    Display,
    Body,
    Caption,
}

impl FontId {
    pub fn get(self) -> &'static Font {
        match self {
            FontId::Icon => &ICON,
            FontId::Micro => &MICRO,
            FontId::Countdown => &COUNTDOWN,
            FontId::Display => &DISPLAY,
            FontId::Body => &BODY,
            FontId::Caption => &CAPTION,
        }
    }
}
