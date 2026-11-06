//! Post-processing: remove flicker from a clip that was recorded without sync.
//!
//! Two separate artefacts, two fixes:
//! - Temporal flicker: every frame is brighter or dimmer depending on how much of
//!   the flash its exposure window caught. Fix = divide each frame by its mean.
//! - Banding: a rolling-shutter frame samples the flicker row by row, so one frame
//!   carries a bright/dark stripe pattern. Fix = divide each row by the flicker
//!   waveform, estimated from the profile itself.

use crate::pwm::estimate_f;

/// Per-frame gain that flattens temporal flicker.
///
/// `means[i]` is the mean luminance of frame `i`. The gain is the local mean
/// divided by the frame mean, so a frame that caught a flash is pulled down to
/// the level of its neighbours. Gains are clamped: a frame that caught almost
/// no light cannot be recovered without amplifying noise.
pub fn temporal_gain(means: &[f64], window: usize, max_gain: f64) -> Vec<f64> {
    let w = window.max(3) | 1;
    let half = w / 2;
    let n = means.len();
    if n == 0 {
        return Vec::new();
    }
    (0..n)
        .map(|i| {
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(n);
            let local = means[lo..hi].iter().sum::<f64>() / (hi - lo) as f64;
            if means[i] <= 0.0 {
                return 1.0;
            }
            (local / means[i]).min(max_gain).max(1.0 / max_gain)
        })
        .collect()
}

/// Per-row gain that flattens banding in one frame.
///
/// `profile` is the per-row luminance of the frame. The flicker frequency is
/// estimated from the profile; the waveform is the detrended profile smoothed
/// over the flicker period, so the gain follows the stripe pattern rather than
/// noise. Rows where the model says the flash was off cannot be recovered: the
/// gain is clamped.
pub fn row_gain(profile: &[f64], line_rate_hz: f64, max_gain: f64) -> (Vec<f64>, f64) {
    if profile.len() < 8 || line_rate_hz <= 0.0 {
        return (vec![1.0; profile.len()], 0.0);
    }
    let Some((f_hz, _)) = estimate_f(profile, line_rate_hz, 100.0, 5000.0) else {
        return (vec![1.0; profile.len()], 0.0);
    };
    // The flicker is multiplicative: row i reads DC * w[i mod p], where w is the
    // waveform over one period of p rows. Averaging the profile over all rows that
    // share a phase gives w (plus the DC). Dividing the profile by w normalized to
    // mean 1 leaves the flat DC. A moving average over one period averages the wave
    // to ~0, so subtracting it leaves the profile unchanged.
    let p = (line_rate_hz / f_hz).round().max(1.0) as usize;
    let n = profile.len();
    let mut wave = vec![0.0; p];
    let mut count = vec![0usize; p];
    for (i, v) in profile.iter().enumerate() {
        wave[i % p] += v;
        count[i % p] += 1;
    }
    for j in 0..p {
        wave[j] /= count[j].max(1) as f64;
    }
    let mean = wave.iter().sum::<f64>() / p as f64;
    let gains = (0..n)
        .map(|i| {
            if wave[i % p] <= 0.01 {
                return 1.0;
            }
            (mean / wave[i % p]).min(max_gain).max(1.0 / max_gain)
        })
        .collect();
    (gains, f_hz)
}


/// Applies row-wise and temporal gains from a source plane into a destination plane,
/// honouring row and pixel strides.
pub fn apply_plane_gains(
    src: &[u8],
    dst: &mut [u8],
    width: usize,
    height: usize,
    src_row_stride: usize,
    src_pixel_stride: usize,
    dst_row_stride: usize,
    dst_pixel_stride: usize,
    row_gains: Option<&[f64]>,
    temporal_gain: f64,
) {
    if width == 0 || height == 0 || src.is_empty() || dst.is_empty() {
        return;
    }

    let is_identity = temporal_gain == 1.0 && row_gains.is_none();

    // Fast path: pure copy without gains, contiguous pixel stride
    if is_identity && src_pixel_stride == 1 && dst_pixel_stride == 1 {
        let copy_w = width.min(src_row_stride).min(dst_row_stride);
        for y in 0..height {
            let s_off = y * src_row_stride;
            let d_off = y * dst_row_stride;
            if s_off + copy_w <= src.len() && d_off + copy_w <= dst.len() {
                dst[d_off..d_off + copy_w].copy_from_slice(&src[s_off..s_off + copy_w]);
            }
        }
        return;
    }

    // Contiguous pixel strides with gain application
    if src_pixel_stride == 1 && dst_pixel_stride == 1 {
        let mut lut = [0u8; 256];
        let mut last_gain = f64::NAN;

        for y in 0..height {
            let g = row_gains.and_then(|rg| rg.get(y)).copied().unwrap_or(1.0) * temporal_gain;
            if g != last_gain {
                for i in 0..256 {
                    lut[i] = (i as f64 * g).round().clamp(0.0, 255.0) as u8;
                }
                last_gain = g;
            }

            let s_off = y * src_row_stride;
            let d_off = y * dst_row_stride;
            if s_off + width <= src.len() && d_off + width <= dst.len() {
                let s_row = &src[s_off..s_off + width];
                let d_row = &mut dst[d_off..d_off + width];
                for (s, d) in s_row.iter().zip(d_row.iter_mut()) {
                    *d = lut[*s as usize];
                }
            }
        }
        return;
    }

    // Strided path (e.g. interleaved UV pixel_stride == 2)
    let mut lut = [0u8; 256];
    let mut last_gain = f64::NAN;

    for y in 0..height {
        let g = row_gains.and_then(|rg| rg.get(y)).copied().unwrap_or(1.0) * temporal_gain;
        if g != last_gain {
            for i in 0..256 {
                lut[i] = (i as f64 * g).round().clamp(0.0, 255.0) as u8;
            }
            last_gain = g;
        }

        let s_row_off = y * src_row_stride;
        let d_row_off = y * dst_row_stride;

        for x in 0..width {
            let s_idx = s_row_off + x * src_pixel_stride;
            let d_idx = d_row_off + x * dst_pixel_stride;
            if s_idx < src.len() && d_idx < dst.len() {
                dst[d_idx] = lut[src[s_idx] as usize];
            }
        }
    }
}
