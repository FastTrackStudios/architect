//! Cross-renderer DSSIM diff + side-by-side composite.

use architect_story_snapshots::{compare, DiffError};
use dssim_core::Dssim;

#[derive(Debug, Clone)]
pub struct ParityReport {
    pub story: &'static str,
    pub blitz_png: Vec<u8>,
    pub wry_png: Vec<u8>,
    pub composite_png: Vec<u8>,
    pub dssim_score: f64,
    pub threshold: f64,
    pub passed: bool,
}

/// Diff `blitz_png` against `wry_png`. Returns the score along with a
/// side-by-side composite PNG suitable for human review.
///
/// `threshold` should be **larger** than for same-renderer baselines —
/// typically `0.02–0.05` — because the two rasterizers paint
/// differently even when the layout is identical.
pub fn diff_renderers(
    story: &'static str,
    blitz_png: Vec<u8>,
    wry_png: Vec<u8>,
    threshold: f64,
) -> Result<ParityReport, DiffError> {
    // First, run a "compare" pass so we re-use all the dssim plumbing
    // and decode logic from architect-story-snapshots. We treat any non-Ok
    // result as a non-fatal "above threshold" — we still want the
    // composite PNG.
    let (score, passed) = match compare(&blitz_png, &wry_png, threshold) {
        Ok(s) => (s, true),
        Err(DiffError::AboveThreshold { value, .. }) => (value, false),
        Err(e) => return Err(e),
    };

    let composite = build_side_by_side(&blitz_png, &wry_png)?;

    Ok(ParityReport {
        story,
        blitz_png,
        wry_png,
        composite_png: composite,
        dssim_score: score,
        threshold,
        passed,
    })
}

/// Decode each PNG to RGBA8 and emit a side-by-side composite (Blitz
/// on the left, wry on the right, separated by a 2px black gutter).
fn build_side_by_side(left: &[u8], right: &[u8]) -> Result<Vec<u8>, DiffError> {
    let (a, aw, ah) = decode_rgba(left)?;
    let (b, bw, bh) = decode_rgba(right)?;

    // Pad to common height (taller of the two), keep widths separate.
    let h = ah.max(bh);
    let gutter = 2u32;
    // Checked, not wrapping: `aw`/`bw` come out of a PNG header, so a
    // crafted (or merely enormous) input must be a decode error rather
    // than a wrapped width and an out-of-bounds blit.
    let total_w = aw
        .checked_add(gutter)
        .and_then(|w| w.checked_add(bw))
        .ok_or_else(|| DiffError::Unsupported(format!("composite width overflows: {aw}+{bw}")))?;

    let mut out = vec![0u8; rgba_len(total_w, h)?];

    blit(&a, aw, ah, &mut out, total_w, 0, 0);
    fill_rect(&mut out, total_w, aw, 0, gutter, h, [0, 0, 0, 255]);
    blit(&b, bw, bh, &mut out, total_w, aw.saturating_add(gutter), 0);

    let mut png = Vec::with_capacity(out.len() / 2);
    {
        let mut encoder = png::Encoder::new(&mut png, total_w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // PNG encoding errors are not DiffError variants; treat as
        // unsupported for caller-facing logging. Realistically these
        // never fire for in-memory RGBA8 output, so the indirection
        // is just to keep the error type contained.
        let mut writer = encoder
            .write_header()
            .map_err(|e| DiffError::Unsupported(format!("png encode: {e}")))?;
        writer
            .write_image_data(&out)
            .map_err(|e| DiffError::Unsupported(format!("png encode: {e}")))?;
        writer
            .finish()
            .map_err(|e| DiffError::Unsupported(format!("png encode: {e}")))?;
    }
    Ok(png)
}

/// Byte length of a `w × h` RGBA8 buffer, or a decode error.
///
/// PNG dimensions are attacker-controlled: `w * h * 4` in `u32` wraps on
/// a large-enough header, and the wrapped value then sizes a buffer that
/// every later offset overruns.
fn rgba_len(w: u32, h: u32) -> Result<usize, DiffError> {
    usize::try_from(w)
        .ok()
        .and_then(|w| w.checked_mul(usize::try_from(h).ok()?))
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| DiffError::Unsupported(format!("image of {w}x{h} is too large")))
}

fn decode_rgba(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), DiffError> {
    let decoder = png::Decoder::new(bytes);
    let mut reader = decoder.read_info().map_err(map_png_err)?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(map_png_err)?;
    buf.truncate(info.buffer_size());
    let (out, w, h) = match info.color_type {
        png::ColorType::Rgba => (buf, info.width, info.height),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(rgba_len(info.width, info.height)?);
            for chunk in buf.as_chunks::<3>().0 {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            (rgba, info.width, info.height)
        }
        png::ColorType::Grayscale => {
            let mut rgba = Vec::with_capacity(rgba_len(info.width, info.height)?);
            for &g in &buf {
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
            (rgba, info.width, info.height)
        }
        png::ColorType::GrayscaleAlpha => {
            let mut rgba = Vec::with_capacity(rgba_len(info.width, info.height)?);
            for chunk in buf.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
            (rgba, info.width, info.height)
        }
        other @ png::ColorType::Indexed => {
            return Err(DiffError::Unsupported(format!(
                "unsupported PNG color type {other:?}"
            )));
        }
    };
    Ok((out, w, h))
}

const fn map_png_err(e: png::DecodingError) -> DiffError {
    DiffError::PngDecode(e)
}

/// Byte offset of pixel `(x, y)` in a `dw`-wide RGBA8 buffer.
fn px_off(dw: u32, x: u32, y: u32) -> Option<usize> {
    let index = usize::try_from(y)
        .ok()?
        .checked_mul(usize::try_from(dw).ok()?)?
        .checked_add(usize::try_from(x).ok()?)?;
    index.checked_mul(4)
}

/// Copy `src` into `dst` at `(x, y)`.
///
/// Rows that would fall outside either buffer are skipped rather than
/// panicking: a composite is a debugging artefact, and a clipped image is
/// a far better outcome than a dead snapshot run.
fn blit(src: &[u8], sw: u32, sh: u32, dst: &mut [u8], dw: u32, x: u32, y: u32) {
    let Ok(row_bytes) = rgba_len(sw, 1) else {
        return;
    };
    for row in 0..sh {
        let (Some(src_off), Some(dst_off)) =
            (px_off(sw, 0, row), px_off(dw, x, y.saturating_add(row)))
        else {
            return;
        };
        let (Some(src_end), Some(dst_end)) = (
            src_off.checked_add(row_bytes),
            dst_off.checked_add(row_bytes),
        ) else {
            return;
        };
        let (Some(src_row), Some(dst_row)) =
            (src.get(src_off..src_end), dst.get_mut(dst_off..dst_end))
        else {
            return;
        };
        dst_row.copy_from_slice(src_row);
    }
}

/// Fill a `w × h` rectangle at `(x, y)` with one colour.
fn fill_rect(dst: &mut [u8], dw: u32, x: u32, y: u32, w: u32, h: u32, rgba: [u8; 4]) {
    for row in 0..h {
        for col in 0..w {
            let Some(off) = px_off(dw, x.saturating_add(col), y.saturating_add(row)) else {
                return;
            };
            let Some(end) = off.checked_add(4) else {
                return;
            };
            if let Some(px) = dst.get_mut(off..end) {
                px.copy_from_slice(&rgba);
            }
        }
    }
}

// Silence the unused-import lint: dssim_core::Dssim is referenced via
// architect_story_snapshots::compare internally; keeping the use here makes
// the dependency intent explicit.
#[allow(dead_code)]
const fn _dssim_dep_marker(_: &Dssim) {}
