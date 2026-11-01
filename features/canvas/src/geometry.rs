//! Points, sizes, rectangles and colours: in a view's points, from its top
//! left.

/// A point, from the top left of what it is in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// This point, moved by `by`.
    #[must_use]
    pub fn offset(self, by: Self) -> Self {
        Self::new(self.x + by.x, self.y + by.y)
    }
}

/// A width and a height.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

impl Size {
    #[must_use]
    pub const fn new(w: f32, h: f32) -> Self {
        Self { w, h }
    }
}

/// A rectangle: its top left corner and its size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    #[must_use]
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    #[must_use]
    pub const fn origin(&self) -> Point {
        Point::new(self.x, self.y)
    }

    /// Whether `p` is inside: its left and top edges are, its right and
    /// bottom are not.
    #[must_use]
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.right() && p.y < self.bottom()
    }
}

/// A colour, `0xRRGGBBAA` — `ReaImGui`'s (and Dear `ImGui`'s `IM_COL32`, read
/// big-endian) way to pass one as a number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color(pub u32);

impl Color {
    pub const TRANSPARENT: Self = Self(0);
    pub const WHITE: Self = Self(0xFFFF_FFFF);

    /// From its red, green and blue, opaque.
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 0xFF)
    }

    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self(u32::from_be_bytes([r, g, b, a]))
    }

    /// Its red, green, blue and alpha.
    #[must_use]
    pub const fn channels(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }

    /// This colour at `alpha` (0..=1) of its opacity.
    #[must_use]
    pub fn faded(self, alpha: f32) -> Self {
        let [r, g, b, a] = self.channels();
        let faded = (f32::from(a) * alpha.clamp(0.0, 1.0)).round();
        // In 0..=255 after the clamp and the round.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::as_conversions
        )]
        Self::rgba(r, g, b, faded as u8)
    }

    /// As CSS: `rgba(r,g,b,a)`.
    #[must_use]
    pub fn css(self) -> String {
        let [r, g, b, a] = self.channels();
        let alpha = f32::from(a) / 255.0;
        format!("rgba({r},{g},{b},{alpha:.3})")
    }

    /// From CSS: `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(…)` or `rgba(…)` —
    /// what a theme gives. `None` for anything else.
    #[must_use]
    pub fn from_css(css: &str) -> Option<Self> {
        let css = css.trim();
        if let Some(hex) = css.strip_prefix('#') {
            return from_hex(hex);
        }
        let inner = css
            .strip_prefix("rgba(")
            .or_else(|| css.strip_prefix("rgb("))?
            .strip_suffix(')')?;
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        let channel = |s: &str| s.parse::<u8>().ok();
        match parts.as_slice() {
            [r, g, b] => Some(Self::rgb(channel(r)?, channel(g)?, channel(b)?)),
            [r, g, b, a] => {
                let a = a.parse::<f32>().ok()?.clamp(0.0, 1.0);
                Some(Self::rgb(channel(r)?, channel(g)?, channel(b)?).faded(a))
            }
            _ => None,
        }
    }
}

fn from_hex(hex: &str) -> Option<Color> {
    let byte = |s: &str| u8::from_str_radix(s, 16).ok();
    let doubled = |c: char| byte(&format!("{c}{c}"));
    let mut chars = hex.chars();
    match hex.len() {
        3 if hex.is_ascii() => Some(Color::rgb(
            doubled(chars.next()?)?,
            doubled(chars.next()?)?,
            doubled(chars.next()?)?,
        )),
        6 | 8 if hex.is_ascii() => {
            let at = |i: usize| hex.get(i..i.saturating_add(2)).and_then(byte);
            let alpha = if hex.len() == 8 { at(6)? } else { 0xFF };
            Some(Color::rgba(at(0)?, at(2)?, at(4)?, alpha))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Color, Point, Rect};

    #[test]
    fn a_rect_holds_its_top_left_edges_not_its_bottom_right() {
        let r = Rect::new(10.0, 10.0, 5.0, 5.0);
        assert!(r.contains(Point::new(10.0, 10.0)));
        assert!(!r.contains(Point::new(15.0, 12.0)));
        assert!(!r.contains(Point::new(9.9, 12.0)));
    }

    #[test]
    fn a_colour_reads_from_a_theme_and_writes_as_css() {
        assert_eq!(Color::from_css("#2dd4bf"), Some(Color(0x2DD4_BFFF)));
        assert_eq!(Color::from_css("#fff"), Some(Color::WHITE));
        assert_eq!(Color::from_css("#11223344"), Some(Color(0x1122_3344)));
        assert_eq!(
            Color::from_css("rgba(255,255,255,0.10)"),
            Some(Color(0xFFFF_FF1A))
        );
        assert_eq!(Color::from_css("0 6px 20px black"), None);
        assert_eq!(Color(0xFF80_00FF).css(), "rgba(255,128,0,1.000)");
    }
}
