//! Cross-renderer DSSIM diff + side-by-side composite.

use dssim_core::Dssim;
use architect_story_snapshots::{compare, DiffError};

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
    let total_w = aw + gutter + bw;

    let mut out = vec![0u8; (total_w * h * 4) as usize];

    blit(&a, aw, ah, &mut out, total_w, h, 0, 0);
    fill_rect(&mut out, total_w, h, aw, 0, gutter, h, [0, 0, 0, 255]);
    blit(&b, bw, bh, &mut out, total_w, h, aw + gutter, 0);

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

fn decode_rgba(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), DiffError> {
    let decoder = png::Decoder::new(bytes);
    let mut reader = decoder.read_info().map_err(map_png_err)?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(map_png_err)?;
    buf.truncate(info.buffer_size());
    let (out, w, h) = match info.color_type {
        png::ColorType::Rgba => (buf, info.width, info.height),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for chunk in buf.chunks_exact(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            (rgba, info.width, info.height)
        }
        png::ColorType::Grayscale => {
            let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for &g in &buf {
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
            (rgba, info.width, info.height)
        }
        png::ColorType::GrayscaleAlpha => {
            let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for chunk in buf.chunks_exact(2) {
                rgba.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
            (rgba, info.width, info.height)
        }
        other => {
            return Err(DiffError::Unsupported(format!(
                "unsupported PNG color type {other:?}"
            )));
        }
    };
    Ok((out, w, h))
}

fn map_png_err(e: png::DecodingError) -> DiffError {
    DiffError::PngDecode(e)
}

#[allow(clippy::too_many_arguments)]
fn blit(src: &[u8], sw: u32, sh: u32, dst: &mut [u8], dw: u32, _dh: u32, x: u32, y: u32) {
    for row in 0..sh {
        let src_off = (row * sw * 4) as usize;
        let dst_off = (((y + row) * dw + x) * 4) as usize;
        let end = src_off + (sw * 4) as usize;
        dst[dst_off..dst_off + (sw * 4) as usize].copy_from_slice(&src[src_off..end]);
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_rect(dst: &mut [u8], dw: u32, _dh: u32, x: u32, y: u32, w: u32, h: u32, rgba: [u8; 4]) {
    for row in 0..h {
        for col in 0..w {
            let off = (((y + row) * dw + (x + col)) * 4) as usize;
            dst[off..off + 4].copy_from_slice(&rgba);
        }
    }
}

// Silence the unused-import lint: dssim_core::Dssim is referenced via
// architect_story_snapshots::compare internally; keeping the use here makes
// the dependency intent explicit.
#[allow(dead_code)]
fn _dssim_dep_marker(_: &Dssim) {}
