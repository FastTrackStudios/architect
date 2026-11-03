//! Validated canvas dimensions in physical pixels.

use crate::Geometry;

pub(super) fn physical_size(geometry: Geometry) -> Option<(u16, u16)> {
    if !geometry.scale.is_finite() || geometry.scale <= 0.0 {
        return None;
    }
    Some((
        dimension(geometry.width, geometry.scale)?,
        dimension(geometry.height, geometry.scale)?,
    ))
}

fn dimension(value: f32, scale: f32) -> Option<u16> {
    let pixels = (f64::from(value) * f64::from(scale)).ceil();
    if !pixels.is_finite() || pixels < 1.0 || pixels > f64::from(u16::MAX) {
        return None;
    }
    #[allow(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    Some(pixels as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_size_rounds_up_and_rejects_invalid_geometry() {
        let size = Geometry {
            width: 100.25,
            height: 30.0,
            scale: 2.0,
        };
        assert_eq!(physical_size(size), Some((201, 60)));
        for invalid in [
            Geometry { scale: 0.0, ..size },
            Geometry {
                scale: f32::NAN,
                ..size
            },
            Geometry {
                width: f32::INFINITY,
                ..size
            },
            Geometry {
                height: -1.0,
                ..size
            },
            Geometry {
                width: 65_536.0,
                scale: 1.0,
                ..size
            },
        ] {
            assert_eq!(physical_size(invalid), None);
        }
    }
}
