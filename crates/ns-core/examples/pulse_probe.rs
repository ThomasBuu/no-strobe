//! Pulse-time fit accuracy on a dir of PGM frames.
//!
//! The frame clock quantizes pulse times to the frame period, so the pulse fit
//! is the accuracy limit for the strobe mode. This prints the pulses, the fit,
//! and the residual per pulse.

use ns_core::detector::{coarse_period, fit_from_pulses};
use ns_core::signal::detrend;

fn pgm_mean(bytes: &[u8]) -> f64 {
    let mut pos = 2;
    let mut toks = Vec::new();
    while toks.len() < 3 {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let end = bytes[pos..].iter().position(|b| b.is_ascii_whitespace()).unwrap();
        toks.push(String::from_utf8_lossy(&bytes[pos..pos + end]).parse::<usize>().unwrap());
        pos += end;
    }
    let raster = &bytes[pos + 1..];
    let sum: u64 = raster.iter().map(|v| *v as u64).sum();
    sum as f64 / (raster.len() as f64 * 255.0)
}

fn main() {
    let dir = std::env::args().nth(1).expect("frames dir");
    let fps: f64 = std::env::args()
        .nth(2)
        .unwrap_or("60".to_string())
        .parse()
        .unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "pgm").unwrap_or(false))
        .collect();
    files.sort();
    let luma: Vec<f64> = files
        .iter()
        .map(|p| pgm_mean(&std::fs::read(p).expect("read pgm")))
        .collect();
    let frame_us = 1e6 / fps;
    let d = detrend(&luma, 15);
    let mean = luma.iter().sum::<f64>() / luma.len() as f64;
    let var = luma.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / luma.len() as f64;
    let thr = mean + 0.6 * var.sqrt();
    let mut pulses: Vec<f64> = Vec::new();
    let mut above = false;
    for (i, v) in luma.iter().enumerate() {
        let now = *v > thr;
        if now && !above {
            pulses.push(i as f64 * frame_us);
        }
        above = now;
    }
    println!("{} frames, {} edges, threshold {:.3}", luma.len(), pulses.len(), thr);
    let coarse = coarse_period(&d, frame_us, 4.0, (fps / 2.0 * 0.8).min(60.0));
    if let Some((t, c)) = coarse {
        println!("coarse T {t:.1} us  corr {c:.3}");
    }
    if let Some(est) = fit_from_pulses(&pulses, coarse.map(|x| x.0).unwrap_or(frame_us), frame_us) {
        println!("fit T {:.1} us  phase {:.1}  conf {:.3}", est.period_us, est.phase_us, est.confidence);
        for p in &pulses {
            let m = ((p - est.phase_us) / est.period_us).round();
            println!("  pulse {p:>9.1}  cycle {m:>4.0}  resid {:>8.1}", p - est.phase_us - m * est.period_us);
        }
    }
}
