use ns_core::signal::SignalBuffer;
use ns_core::signal::Frame;
use ns_core::detector::{coarse_period, fit_from_pulses};

fn synth(t_us: f64, exposure_us: f64, period_us: f64, phase_us: f64, flash_us: f64) -> f64 {
    let start = t_us - exposure_us;
    let mut lit = 0.0;
    let mut cycle = ((start - phase_us) / period_us).floor();
    while cycle * period_us + phase_us < t_us {
        let pulse_start = cycle * period_us + phase_us;
        let pulse_end = pulse_start + flash_us;
        let overlap = (t_us.min(pulse_end) - start.max(pulse_start)).max(0.0);
        lit += overlap;
        cycle += 1.0;
    }
    0.2 + 0.8 * lit / exposure_us
}

fn main() {
    let period_us = 100_000.0;
    let fps = 30.0;
    let frame_us = 1e6 / fps;

    let mut buf = SignalBuffer::new(120);
    for i in 0..45 {
        let t_us = i as f64 * frame_us;
        buf.push(Frame { t_us, luminance: synth(t_us, 8_000.0, period_us, 30_000.0, 10_000.0) });
    }

    let det = buf.detrended(15);
    let coarse = coarse_period(&det, buf.frame_period_us(), 4.0, 40.0);
    println!("coarse = {:?}", coarse);

    let lum: Vec<f64> = buf.frames().map(|f| f.luminance).collect();
    let mean = lum.iter().sum::<f64>() / lum.len() as f64;
    let std = (lum.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / lum.len() as f64).sqrt();
    let pulses = buf.pulse_times(mean + 0.6 * std);
    println!("pulses = {:?}", pulses);
    println!("fit = {:?}", fit_from_pulses(&pulses, coarse.map(|(p, _)| p).unwrap_or(2.0 * frame_us), frame_us));
}
