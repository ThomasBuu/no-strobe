//! LED / PWM flicker: measure the flicker frequency from the vertical
//! luminance profile of a frame, then pick an exposure that is an integer
//! multiple of the flicker period.
//!
//! A rolling-shutter sensor reads rows one after another, so a frame is a
//! time scan at the row readout rate. A PWM-driven light at `f` Hz shows up as
//! banding in the vertical profile, sampled at `line_rate_hz` rows per second.
//! The frame-mean luminance is useless here: at 100 Hz and above it aliases.

use crate::signal::detrend;

/// Magnitude spectrum of a real signal, padded to the next power of two.
pub fn fft_mag(x: &[f64]) -> Vec<f64> {
    let n = x.len().next_power_of_two();
    let mut re = vec![0.0f64; n];
    let mut im = vec![0.0f64; n];
    for (i, v) in x.iter().enumerate() {
        re[i] = *v;
    }
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2usize;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        let half = len / 2;
        let mut i = 0;
        while i < n {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..half {
                let (ur, ui) = (re[i + k], im[i + k]);
                let (vr, vi) = (re[i + k + half], im[i + k + half]);
                let nr = ur * cr - ui * ci;
                let ni = ur * ci + ui * cr;
                re[i + k] = ur + nr;
                im[i + k] = ui + ni;
                re[i + k + half] = vr - nr;
                im[i + k + half] = vi - ni;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
            i += len;
        }
        len <<= 1;
    }
    re.iter().zip(im.iter()).map(|(r, i)| (r * r + i * i).sqrt()).collect()
}

/// Flicker frequency from the vertical profile.
///
/// `line_rate_hz` is the row readout rate (rows per second). A flicker at `f` Hz
/// bands with a period of `line_rate_hz / f` rows, so the profile's
/// autocorrelation peaks at that lag. The lag is refined with a parabola, which
/// gives sub-row precision: FFT bins would only resolve `line_rate_hz / N` Hz.
/// Returns `(frequency_hz, peak_over_mean)` — the second value is a confidence
/// proxy: a clean square wave gives a strong peak, a static scene gives none.
pub fn estimate_f(profile: &[f64], line_rate_hz: f64, min_hz: f64, max_hz: f64) -> Option<(f64, f64)> {
    if profile.len() < 16 || line_rate_hz <= 0.0 {
        return None;
    }
    // High-pass: remove the slow lighting gradient, keep the flicker band.
    let d = detrend(profile, profile.len() / 16 | 1);
    let lo_lag = (line_rate_hz / max_hz).max(1.0).ceil() as usize;
    let hi_lag = (line_rate_hz / min_hz).min(d.len() as f64 - 2.0).floor() as usize;
    if hi_lag <= lo_lag {
        return None;
    }
    let energy = d.iter().map(|v| v * v).sum::<f64>();
    if energy <= 1e-12 {
        return None;
    }
    let mut best = lo_lag;
    let mut best_c = 0.0f64;
    for lag in lo_lag..=hi_lag {
        let c = autocorr(&d, lag) / energy;
        if c > best_c {
            best_c = c;
            best = lag;
        }
    }
    if best_c < 0.3 {
        return None;
    }
    let (a, b, c) = (autocorr(&d, best - 1) / energy, best_c, autocorr(&d, best + 1) / energy);
    let denom = a - 2.0 * b + c;
    let shift = if denom.abs() > 1e-12 { (a - c) / (2.0 * denom) } else { 0.0 };
    let lag = best as f64 + shift.clamp(-1.0, 1.0);
    if lag <= 0.0 {
        return None;
    }
    Some((line_rate_hz / lag, best_c))
}

fn autocorr(x: &[f64], lag: usize) -> f64 {
    x.iter()
        .zip(x.iter().skip(lag))
        .map(|(a, b)| a * b)
        .sum()
}

/// Banding period in rows, from the profile's autocorrelation.
///
/// The rows of a frame are read in `frame_us`, so `rows` rows span the frame time and one row
/// takes `frame_us / rows`. A period of `p` rows is a flicker period of `p * frame_us / rows`
/// microseconds. The row time is never measured: it cancels between the profile and the frame.
pub fn estimate_period_rows(profile: &[f64], min_rows: f64, max_rows: f64) -> Option<(f64, f64)> {
    let n = profile.len();
    if n < 8 {
        return None;
    }
    let mean = profile.iter().sum::<f64>() / n as f64;
    let x: Vec<f64> = profile.iter().map(|v| v - mean).collect();
    let lo = (min_rows.ceil() as usize).max(2);
    let hi = (max_rows.floor() as usize).min(n / 2);
    if lo > hi {
        return None;
    }
    let norm = x.iter().map(|v| v * v).sum::<f64>();
    if norm <= 0.0 {
        return None;
    }
    let mut c = Vec::with_capacity(hi - lo + 1);
    for lag in lo..=hi {
        c.push(autocorr(&x, lag) / norm);
    }
    if c.len() < 3 {
        return None;
    }
    // The correlation of a smooth profile is high at small lags: two rows inside one bright
    // pulse read the same light. The period is not the global maximum, it is the first positive
    // peak after the profile has gone negative - the lag where the pattern lines up with itself
    // once. A local maximum inside the negative plateau (the shallow wiggles of a flat region)
    // is not a period: the peak must be positive, and the trough before it negative.
    let mut trough = None;
    for i in 0..c.len() - 1 {
        if c[i] < 0.0 {
            trough = Some(i);
            break;
        }
    }
    let start = match trough {
        Some(i) => i + 1,
        // Never negative: the profile is smooth (no banding), no period to find.
        None => return None,
    };
    let mut peak = None;
    for i in start..c.len() - 1 {
        if c[i] > 0.0 && c[i] >= c[i - 1] && c[i] >= c[i + 1] {
            peak = Some(i);
            break;
        }
    }
    let idx = match peak {
        Some(i) => i,
        // No positive peak inside the range: the period is longer than the frame shows.
        None => return None,
    };
    let best_c = c[idx];
    if best_c <= 0.0 {
        return None;
    }
    // A real period anti-correlates at half its length: shifting the row scan by half a flicker
    // period turns every bright pulse dark. Noise has no such structure, so a peak found in a
    // profile that is mostly noise (a long exposure integrating many cycles) is rejected here.
    let half = (idx + lo) / 2;
    if half >= 1 {
        let c_half = autocorr(&x, half) / norm;
        if c_half > -0.25 * best_c {
            return None;
        }
    }
    let left = if idx > 0 { c[idx - 1] } else { best_c };
    let right = if idx + 1 < c.len() { c[idx + 1] } else { best_c };
    let denom = left - 2.0 * best_c + right;
    let shift = if denom.abs() > 1e-12 { (left - right) / (2.0 * denom) } else { 0.0 };
    let p = (idx as f64 + lo as f64 + shift).clamp(lo as f64, hi as f64);
    Some((p, best_c))
}

/// Exposure that integrates `k` whole flicker cycles, in microseconds.
pub fn exposure_for_k(f_hz: f64, k: u32) -> f64 {
    1e6 * k as f64 / f_hz
}

/// Smallest `k` whose exposure fits the device range and the frame period.
pub fn pick_k(f_hz: f64, limits: &crate::camera::CameraLimits, fps: f64) -> Option<(u32, f64)> {
    let frame_us = 1e6 / fps;
    for k in 1..64u32 {
        let e = exposure_for_k(f_hz, k);
        if e < limits.min_exposure_us {
            continue;
        }
        if e > limits.max_exposure_us {
            return None;
        }
        if e + limits.sustain_us <= frame_us {
            return Some((k, e));
        }
    }
    None
}

/// Banding amplitude scaled by the exposure that produced the profile.
///
/// A row integrates the flicker over a window `e`, so the profile is the square
/// wave convolved with a box of width `e`: the banding amplitude is
/// `A * |sin(pi * e * f)| / (pi * f * e)`. Raw amplitude therefore decays with
/// `e` and a search on it walks to the longest exposure. Multiplying by `e`
/// removes the decay and leaves isolated minima exactly at `e = k / f`.
pub fn banding_metric(profile: &[f64], exposure_us: f64) -> f64 {
    banding_amplitude(profile) * exposure_us
}

/// Flicker as change *between frames*: for each row the standard deviation of its value over
/// `frames` (same exposure, phases that differ), averaged over the rows and divided by the mean
/// luma. Anything static along the rows (the lamp, the cables, the walls) cancels; flicker,
/// whose phase moves from frame to frame, stays. Zero (to the noise) at an exposure that
/// integrates whole periods. Measured on a real lamp (100 Hz): 0.074 at 4 ms against 0.009 at
/// 10 ms, while the spatial `banding_amplitude` stayed flat (0.022) because the scene structure
/// dominated. Blind spot: a flicker whose frequency is a whole multiple of the frame rate has the
/// same phase in every frame.
pub fn temporal_banding(frames: &[Vec<f64>]) -> f64 {
    let m = frames.len();
    if m < 2 {
        return 0.0;
    }
    let n = frames.iter().map(|f| f.len()).min().unwrap_or(0);
    if n == 0 {
        return 0.0;
    }
    let mut sum_std = 0.0;
    let mut sum_mean = 0.0;
    for i in 0..n {
        let mu = frames.iter().map(|f| f[i]).sum::<f64>() / m as f64;
        let var = frames.iter().map(|f| (f[i] - mu) * (f[i] - mu)).sum::<f64>() / m as f64;
        sum_std += var.sqrt();
        sum_mean += mu;
    }
    let mean = sum_mean / n as f64;
    if mean <= 1e-9 { 0.0 } else { (sum_std / n as f64) / mean }
}

/// Banding amplitude of a profile: standard deviation after detrending.
/// Zero means the exposure integrates a whole number of flicker cycles.
pub fn banding_amplitude(profile: &[f64]) -> f64 {
    banding_amplitude_window(profile, profile.len() / 4 | 1)
}

/// Same, with the detrend window in rows. A window shorter than a few flicker periods eats
/// part of the banding itself, so the amplitude of a short exposure is not comparable to the
/// amplitude of a long one: the scan needs a window of several periods.
pub fn banding_amplitude_window(profile: &[f64], window: usize) -> f64 {
    if profile.len() < 4 {
        return 0.0;
    }
    let d = detrend(profile, window.max(1) | 1);
    let m = d.iter().sum::<f64>() / d.len() as f64;
    (d.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / d.len() as f64).sqrt()
}

/// Vertical luminance profile of a synthetic flicker, for tests and examples.
///
/// Row `r` is read at `r / line_rate_hz` seconds and integrates a square wave at
/// `f_hz` (duty `duty`, phase `phase_s`) over its exposure window. The overlap
/// is computed exactly: sampling the square wave on a grid leaves a noise floor
/// the same size as the banding at long exposures, which hides the zeros the
/// closed loop looks for.
pub fn synthetic_profile(
    exposure_us: f64,
    f_hz: f64,
    phase_s: f64,
    duty: f64,
    line_rate_hz: f64,
    rows: usize,
) -> Vec<f64> {
    let cycles = exposure_us / 1e6 * f_hz;
    (0..rows)
        .map(|r| {
            let start = (r as f64 / line_rate_hz * f_hz + phase_s) % 1.0 - cycles;
            0.2 + 0.8 * flash_fraction(start, cycles, duty)
        })
        .collect()
}

/// Fraction of a window of `cycles` flicker cycles, starting at cycle phase
/// `start` (which may be negative), that lands inside the flash part `[0, duty)`.
pub fn flash_fraction(start: f64, cycles: f64, duty: f64) -> f64 {
    let whole = cycles.floor();
    let rem = cycles - whole;
    let mut u = start % 1.0;
    if u < 0.0 {
        u += 1.0;
    }
    // Whole cycles contribute `duty` each. The remainder is an arc of length
    // `rem` starting at phase `u`; it may wrap past 1.0.
    let mut hit = whole * duty;
    if u + rem <= 1.0 {
        if u < duty {
            hit += (u + rem).min(duty) - u;
        }
    } else {
        if u < duty {
            hit += duty - u;
        }
        hit += (u + rem - 1.0).min(duty);
    }
    (hit / cycles).max(0.0).min(1.0)
}
