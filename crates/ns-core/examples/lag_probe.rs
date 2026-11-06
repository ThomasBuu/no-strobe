//! Prints the autocorrelation peaks of a synthetic profile so the harmonic
//! problem in `estimate_f` is visible.

use ns_core::pwm::flash_fraction;
use ns_core::signal::detrend;

fn profile(exposure_us: f64, f_hz: f64, phase_s: f64, duty: f64, line_rate_hz: f64, rows: usize) -> Vec<f64> {
    let cycles = exposure_us / 1e6 * f_hz;
    (0..rows)
        .map(|r| {
            let start = (r as f64 / line_rate_hz * f_hz + phase_s) % 1.0 - cycles;
            0.2 + 0.8 * flash_fraction(start, cycles, duty)
        })
        .collect()
}

fn autocorr(x: &[f64], lag: usize) -> f64 {
    let n = x.len();
    if lag >= n {
        return 0.0;
    }
    let mut num = 0.0;
    let mut den = 0.0;
    for i in 0..n - lag {
        num += x[i] * x[i + lag];
        den += x[i] * x[i];
    }
    num / den
}

/// Normalization used by `estimate_f`: divide by the energy of the whole
/// profile, so long lags cannot win by shrinking the denominator.
fn norm_autocorr(x: &[f64], lag: usize) -> f64 {
    let n = x.len();
    if lag >= n {
        return 0.0;
    }
    let total: f64 = x.iter().map(|v| v * v).sum();
    let mut num = 0.0;
    for i in 0..n - lag {
        num += x[i] * x[i + lag];
    }
    num / total
}

fn main() {
    for f in [500.0, 1000.0, 2000.0] {
        let p = profile(100.0, f, 0.3, 0.3, 40000.0, 512);
        let d = detrend(&p, 32);
        let mut peaks: Vec<(usize, f64)> = (8..480).map(|lag| (lag, norm_autocorr(&d, lag))).collect();
        peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        print!("f {f}  true lag {:1}  top lags:", (40000.0 / f) as usize);
        for (lag, v) in peaks.iter().take(6) {
            print!("  {lag}:{v:.3}");
        }
        println!();
        let true_lag = (40000.0 / f) as usize;
        println!("   lag {true_lag} = {:.3}", norm_autocorr(&d, true_lag));
    }
}
