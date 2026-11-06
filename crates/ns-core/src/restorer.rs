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
