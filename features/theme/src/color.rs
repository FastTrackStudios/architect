//! Colours: parsed from CSS text, mixed in `OKLab`, measured for contrast.
//!
//! A theme's values are CSS strings (they become CSS variables as they
//! are), so [`Rgba::parse`] reads the forms themes use: `#rgb`, `#rgba`,
//! `#rrggbb`, `#rrggbbaa`, `rgb()`/`rgba()`, `hsl()`/`hsla()`, `oklch()`,
//! `transparent`, `white` and `black`. Mixing is in `OKLab` (perceptually
//! even: half way between two colours looks half way) and contrast is
//! WCAG 2's ratio.

// The colour maths reads as its papers write it: r, g, b, l, m, s, h.
#![allow(clippy::many_single_char_names)]

use std::fmt;

/// An sRGB colour with alpha, each channel 0..=1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// What an unknown token shows as: loud on purpose, so a typo is seen.
pub const MAGENTA: Rgba = Rgba::rgb(1.0, 0.0, 1.0);
pub const WHITE: Rgba = Rgba::rgb(1.0, 1.0, 1.0);
pub const BLACK: Rgba = Rgba::rgb(0.0, 0.0, 0.0);

impl Rgba {
    #[must_use]
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// This colour with alpha `a`.
    #[must_use]
    pub const fn alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// From CSS text; `None` for anything it does not read.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let lower = text.to_ascii_lowercase();
        match lower.as_str() {
            "transparent" => return Some(BLACK.alpha(0.0)),
            "white" => return Some(WHITE),
            "black" => return Some(BLACK),
            _ => {}
        }
        if let Some(hex) = lower.strip_prefix('#') {
            return parse_hex(hex);
        }
        let (name, args) = function(&lower)?;
        match name {
            "rgb" | "rgba" => parse_rgb(args),
            "hsl" | "hsla" => parse_hsl(args),
            "oklch" => parse_oklch(args),
            _ => None,
        }
    }

    /// As CSS: `#rrggbb` when opaque, `rgba(r,g,b,a)` otherwise.
    #[must_use]
    pub fn to_css(self) -> String {
        let [r, g, b] = [self.r, self.g, self.b].map(byte);
        if self.a >= 0.999 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("rgba({r},{g},{b},{:.2})", self.a.clamp(0.0, 1.0))
        }
    }

    /// WCAG 2 relative luminance (alpha ignored).
    #[must_use]
    pub fn luminance(self) -> f32 {
        let [r, g, b] = [self.r, self.g, self.b].map(to_linear);
        0.0722f32.mul_add(b, 0.2126f32.mul_add(r, 0.7152 * g))
    }

    /// Whether it reads as a light colour (dark text goes on it).
    #[must_use]
    pub fn is_light(self) -> bool {
        self.luminance() > 0.18
    }

    /// This colour drawn over `under` (which is taken as opaque).
    #[must_use]
    pub fn over(self, under: Self) -> Self {
        let a = self.a.clamp(0.0, 1.0);
        let mix = |top: f32, bottom: f32| top.mul_add(a, bottom * (1.0 - a));
        Self::rgb(
            mix(self.r, under.r),
            mix(self.g, under.g),
            mix(self.b, under.b),
        )
    }

    /// Mixed with `other` in `OKLab`: `t` 0 is `self`, 1 is `other`.
    #[must_use]
    pub fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let (a, b) = (Oklab::from(self), Oklab::from(other));
        let lerp = |x: f32, y: f32| (y - x).mul_add(t, x);
        let mixed = Oklab {
            l: lerp(a.l, b.l),
            a: lerp(a.a, b.a),
            b: lerp(a.b, b.b),
        };
        mixed.to_rgb().alpha(lerp(self.a, other.a))
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_css())
    }
}

/// WCAG 2 contrast between `ink` (composited over `ground` if it is see
/// through) and `ground`: 1 to 21.
#[must_use]
pub fn contrast(ink: Rgba, ground: Rgba) -> f32 {
    let ground = ground.over(WHITE);
    let ink = ink.over(ground);
    let (x, y) = (ink.luminance(), ground.luminance());
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

/// `ink`, moved toward `toward` in `OKLab` only as far as it takes to reach
/// `min` contrast on every one of `grounds`. Deterministic: it steps by
/// 1/64 and stops at the first step that passes (or at `toward`).
#[must_use]
pub fn with_contrast(ink: Rgba, grounds: &[Rgba], min: f32, toward: Rgba) -> Rgba {
    let passes = |c: Rgba| grounds.iter().all(|g| contrast(c, *g) >= min);
    (0u8..=64)
        .map(|step| ink.mix(toward, f32::from(step) / 64.0))
        .find(|c| passes(*c))
        .unwrap_or(toward)
}

#[derive(Debug, Clone, Copy)]
struct Oklab {
    l: f32,
    a: f32,
    b: f32,
}

impl From<Rgba> for Oklab {
    fn from(c: Rgba) -> Self {
        let [r, g, b] = [c.r, c.g, c.b].map(to_linear);
        let l = 0.051_457_565f32
            .mul_add(b, 0.412_221_46f32.mul_add(r, 0.536_332_55 * g))
            .cbrt();
        let m = 0.107_406_58f32
            .mul_add(b, 0.211_903_5f32.mul_add(r, 0.680_699_5 * g))
            .cbrt();
        let s = 0.629_978_7f32
            .mul_add(b, 0.088_302_46f32.mul_add(r, 0.281_718_85 * g))
            .cbrt();
        Self {
            l: 0.004_072_047f32.mul_add(-s, 0.210_454_26f32.mul_add(l, 0.793_617_8 * m)),
            a: 0.450_593_7f32.mul_add(s, 1.977_998_5f32.mul_add(l, -2.428_592_2 * m)),
            b: 0.808_675_77f32.mul_add(-s, 0.025_904_037f32.mul_add(l, 0.782_771_77 * m)),
        }
    }
}

impl Oklab {
    fn to_rgb(self) -> Rgba {
        let l = 0.215_803_76f32
            .mul_add(self.b, 0.396_337_78f32.mul_add(self.a, self.l))
            .powi(3);
        let m = 0.063_854_17f32
            .mul_add(-self.b, 0.105_561_346f32.mul_add(-self.a, self.l))
            .powi(3);
        let s = 1.291_485_5f32
            .mul_add(-self.b, 0.089_484_18f32.mul_add(-self.a, self.l))
            .powi(3);
        let r = 0.230_759_05f32.mul_add(s, 4.076_741_7f32.mul_add(l, -3.307_711_6 * m));
        let g = 0.341_319_4f32.mul_add(-s, (-1.268_438f32).mul_add(l, 2.609_757_4 * m));
        let b = 1.707_614_7f32.mul_add(s, (-0.004_196_086_3f32).mul_add(l, -0.703_418_6 * m));
        Rgba::rgb(from_linear(r), from_linear(g), from_linear(b))
    }
}

fn to_linear(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055f32.mul_add(c.powf(1.0 / 2.4), -0.055)
    }
}

/// A channel 0..=1 as a byte.
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn byte(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn parse_hex(hex: &str) -> Option<Rgba> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let nibble = |i: usize| -> Option<f32> {
        let d = char::from(*hex.as_bytes().get(i)?).to_digit(16)?;
        Some(f32::from(u8::try_from(d.checked_mul(17)?).ok()?) / 255.0)
    };
    let pair = |i: usize| -> Option<f32> {
        let s = hex.get(i..i.checked_add(2)?)?;
        Some(f32::from(u8::from_str_radix(s, 16).ok()?) / 255.0)
    };
    match hex.len() {
        3 | 4 => Some(Rgba {
            r: nibble(0)?,
            g: nibble(1)?,
            b: nibble(2)?,
            a: if hex.len() == 4 { nibble(3)? } else { 1.0 },
        }),
        6 | 8 => Some(Rgba {
            r: pair(0)?,
            g: pair(2)?,
            b: pair(4)?,
            a: if hex.len() == 8 { pair(6)? } else { 1.0 },
        }),
        _ => None,
    }
}

/// `name(args)` → `(name, args)`.
fn function(text: &str) -> Option<(&str, &str)> {
    let open = text.find('(')?;
    let name = text.get(..open)?.trim();
    let rest = text.get(open.checked_add(1)?..)?;
    let args = rest.strip_suffix(')')?;
    Some((name, args))
}

/// A function's arguments: three or four numbers, by commas or spaces,
/// the alpha after a `/` or as a fourth.
fn numbers(args: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = args
        .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    (3..=4).contains(&parts.len()).then_some(parts)
}

/// A number, or a percentage of `full`.
fn number(text: &str, full: f32) -> Option<f32> {
    text.strip_suffix('%').map_or_else(
        || text.parse::<f32>().ok(),
        |pct| pct.parse::<f32>().ok().map(|p| p / 100.0 * full),
    )
}

fn alpha(parts: &[&str]) -> Option<f32> {
    parts
        .get(3)
        .map_or(Some(1.0), |a| number(a, 1.0).map(|a| a.clamp(0.0, 1.0)))
}

fn parse_rgb(args: &str) -> Option<Rgba> {
    let parts = numbers(args)?;
    let channel = |i: usize| number(parts.get(i)?, 255.0).map(|v| (v / 255.0).clamp(0.0, 1.0));
    Some(Rgba {
        r: channel(0)?,
        g: channel(1)?,
        b: channel(2)?,
        a: alpha(&parts)?,
    })
}

fn degrees(text: &str) -> Option<f32> {
    text.strip_suffix("deg").unwrap_or(text).parse::<f32>().ok()
}

fn parse_hsl(args: &str) -> Option<Rgba> {
    let parts = numbers(args)?;
    let h = degrees(parts.first()?)?.rem_euclid(360.0) / 360.0;
    let s = number(parts.get(1)?, 1.0)?.clamp(0.0, 1.0);
    let l = number(parts.get(2)?, 1.0)?.clamp(0.0, 1.0);
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l.mul_add(-s, l + s)
    };
    let p = 2.0f32.mul_add(l, -q);
    let hue = |mut t: f32| {
        t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            ((q - p) * 6.0).mul_add(t, p)
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            ((q - p) * (2.0 / 3.0 - t)).mul_add(6.0, p)
        } else {
            p
        }
    };
    Some(Rgba {
        r: hue(h + 1.0 / 3.0),
        g: hue(h),
        b: hue(h - 1.0 / 3.0),
        a: alpha(&parts)?,
    })
}

fn parse_oklch(args: &str) -> Option<Rgba> {
    let parts = numbers(args)?;
    let l = number(parts.first()?, 1.0)?;
    let c = number(parts.get(1)?, 0.4)?;
    let h = parts
        .get(2)
        .and_then(|h| degrees(h))
        .unwrap_or(0.0)
        .to_radians();
    let lab = Oklab {
        l,
        a: c * h.cos(),
        b: c * h.sin(),
    };
    Some(lab.to_rgb().alpha(alpha(&parts)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_form_themes_use() {
        assert_eq!(Rgba::parse("#ff00ff"), Some(MAGENTA));
        assert_eq!(
            Rgba::parse("#f0f").map(Rgba::to_css).as_deref(),
            Some("#ff00ff")
        );
        assert_eq!(Rgba::parse("rgb(255, 0, 255)"), Some(MAGENTA));
        assert_eq!(
            Rgba::parse("rgba(255,255,255,0.10)")
                .map(Rgba::to_css)
                .as_deref(),
            Some("rgba(255,255,255,0.10)")
        );
        assert_eq!(
            Rgba::parse("hsl(300 100% 50%)")
                .map(Rgba::to_css)
                .as_deref(),
            Some("#ff00ff")
        );
        assert_eq!(
            Rgba::parse("oklch(1 0 0)").map(Rgba::to_css).as_deref(),
            Some("#ffffff")
        );
        assert_eq!(
            Rgba::parse("oklch(0 0 0)").map(Rgba::to_css).as_deref(),
            Some("#000000")
        );
        assert!(Rgba::parse("oklch(0.6 0.22 28)").is_some());
        assert_eq!(Rgba::parse("0.5rem"), None);
        assert_eq!(Rgba::parse("#ggg"), None);
    }

    #[test]
    fn oklab_round_trips_and_mixes_evenly() {
        for hex in ["#09090b", "#2dd4bf", "#e4e4e7", "#ff00ff", "#808080"] {
            let c = Rgba::parse(hex).unwrap();
            assert_eq!(c.mix(c, 0.5).to_css(), hex);
        }
        assert_eq!(BLACK.mix(WHITE, 0.0).to_css(), "#000000");
        assert_eq!(BLACK.mix(WHITE, 1.0).to_css(), "#ffffff");
    }

    #[test]
    fn contrast_is_wcag() {
        assert!((contrast(BLACK, WHITE) - 21.0).abs() < 0.01);
        assert!((contrast(WHITE, WHITE) - 1.0).abs() < 0.01);
        let grey = Rgba::parse("#777777").unwrap();
        let fixed = with_contrast(grey, &[WHITE], 4.5, BLACK);
        assert!(contrast(fixed, WHITE) >= 4.5);
    }
}
