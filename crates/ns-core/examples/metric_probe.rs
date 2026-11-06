//! Prints the banding metric across exposures so the hill climb's landscape is visible.

use ns_core::pwm;

fn profile_for(exposure_us: f64, f_hz: f64, phase_s: f64, duty: f64, line_rate_hz: f64, rows: usize) -> Vec<f64> {
    pwm::synthetic_profile(exposure_us, f_hz, phase_s, duty, line_rate_hz, rows)
}

fn main() {
    let f = 2000.0;
    let line = 40000.0;
    debug_rows();
    let p = profile_for(26100.0, f, 0.3, 0.3, line, 512);
    let (lo, hi) = spread(&p);
    println!("e 26100.0  lo {lo:.6}  hi {hi:.6}  spread {:.6}", hi - lo);
    for e in (26000..28600).step_by(100) {
        let e = e as f64;
        for phase in [0.0, 0.3, 0.55] {
            let p = profile_for(e, f, phase, 0.3, line, 512);
            let band = ns_core::pwm::banding_amplitude(&p);
            println!("e {:>9.1}  cycles {:>6.3}  band {:.5}  metric {:>8.1}", e, e * f / 1e6, band, band * e);
        }
    }
}

#[allow(dead_code)]
fn spread(p: &[f64]) -> (f64, f64) {
    let lo = p.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = p.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    (lo, hi)
}

#[allow(dead_code)]
fn debug_rows() {
    let f = 2000.0;
    let line = 40000.0;
    let cycles = 26100.0 / 1e6 * f;
    for r in [0usize, 100, 200] {
        let start = (r as f64 / line * f + 0.3) % 1.0 - cycles;
        let mut u = start % 1.0;
        if u < 0.0 { u += 1.0; }
        println!("row {r} start {start:.4} u {u:.4} cycles {cycles:.4} frac {:.6}",
            pwm::flash_fraction(start, cycles, 0.3));
    }
}
