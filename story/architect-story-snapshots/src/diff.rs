//! DSSIM-based perceptual diff of two PNG buffers.
//!
//! Threshold semantics follow `dssim-core`: lower is closer; `0.0` is
//! pixel-perfect identity; `0.001` rejects only changes a human would
//! notice on a side-by-side compare. We default to `0.001` to match
//! `tonybierman/blitz-vrt`'s tuned default.

use dssim_core::{Dssim, DssimImage};

#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    #[error("PNG decode failed: {0}")]
    PngDecode(#[from] png::DecodingError),
    #[error("size mismatch: baseline {baseline:?}, candidate {candidate:?}")]
    SizeMismatch {
        baseline: (u32, u32),
        candidate: (u32, u32),
    },
    #[error("dssim could not build image (likely zero-sized PNG)")]
    DssimImageBuild,
    #[error("dssim score {value:.6} exceeded threshold {threshold:.6}")]
    AboveThreshold { value: f64, threshold: f64 },
    #[error("unsupported input: {0}")]
    Unsupported(String),
}

/// Decode `bytes` as RGBA8.
fn decode_rgba(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), DiffError> {
    let decoder = png::Decoder::new(bytes);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());

    // Promote whatever color type came back to RGBA8.
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
            // ImageMagick `import` against an Xvfb screen sometimes
            // emits Grayscale when the captured region is uniform —
            // typically a too-early screenshot before the WebView
            // has painted. Promote to RGBA so the diff still runs;
            // the resulting score will be poor (uniform image),
            // which is the right signal that paint_grace needs
            // bumping.
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

fn build_image(
    dssim: &Dssim,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<DssimImage<f32>, DiffError> {
    // dssim wants packed `rgb::RGBA<u8>` slices.
    let pixels: Vec<rgb::RGBA8> = rgba
        .chunks_exact(4)
        .map(|c| rgb::RGBA8::new(c[0], c[1], c[2], c[3]))
        .collect();
    dssim
        .create_image_rgba(&pixels, width as usize, height as usize)
        .ok_or(DiffError::DssimImageBuild)
}

/// Compare `candidate` against `baseline`. Returns the dssim score on
/// success so callers can log near-threshold failures.
pub fn compare(baseline: &[u8], candidate: &[u8], threshold: f64) -> Result<f64, DiffError> {
    let (a, aw, ah) = decode_rgba(baseline)?;
    let (b, bw, bh) = decode_rgba(candidate)?;

    if (aw, ah) != (bw, bh) {
        return Err(DiffError::SizeMismatch {
            baseline: (aw, ah),
            candidate: (bw, bh),
        });
    }

    let dssim = Dssim::new();
    let img_a = build_image(&dssim, &a, aw, ah)?;
    let img_b = build_image(&dssim, &b, bw, bh)?;

    let (val, _ssim_maps) = dssim.compare(&img_a, &img_b);
    let score: f64 = val.into();

    if score <= threshold {
        Ok(score)
    } else {
        Err(DiffError::AboveThreshold {
            value: score,
            threshold,
        })
    }
}
