use ns_core::pwm::synthetic_profile;
use ns_core::restorer::{row_gain, temporal_gain};

const LINE_RATE: f64 = 40000.0;
const ROWS: usize = 256;
const F_HZ: f64 = 1000.0;
// Banding is only readable when the exposure window is shorter than one flicker
// cycle. At exposure = 1/f a row integrates a whole cycle and the profile is flat.
const EXPOSURE: f64 = 100.0;

fn spread(xs: &[f64]) -> f64 {
    let m = xs.iter().sum::<f64>() / xs.len() as f64;
    (xs.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / xs.len() as f64).sqrt()
}

#[test]
fn row_gain_flattens_banding() {
    // Phase advances frame by frame: the stripe pattern moves, as in a real clip.
    let mut phase = 0.0;
    let mut before = Vec::new();
    let mut after = Vec::new();
    for _ in 0..20 {
        let p = synthetic_profile(EXPOSURE, F_HZ, phase, 0.3, LINE_RATE, ROWS);
        let (g, f) = row_gain(&p, LINE_RATE, 3.0);
        assert!((f - F_HZ).abs() < 0.02 * F_HZ, "f {f}");
        let fixed: Vec<f64> = p.iter().zip(g.iter()).map(|(v, k)| v * k).collect();
        before.push(spread(&p));
        after.push(spread(&fixed));
        println!("f {f} band {:.5} -> {:.5}", spread(&p), spread(&fixed));
        phase += 0.013;
    }
    let b = before.iter().sum::<f64>() / before.len() as f64;
    let a = after.iter().sum::<f64>() / after.len() as f64;
    println!("mean banding {b} -> {a}");
    assert!(a < b * 0.5, "banding {b} -> {a}");
}

#[test]
fn temporal_gain_flattens_frame_brightness() {
    // Frame means at a 10 Hz strobe: each frame catches a different share of the flash.
    let t_frame = 1e6 / 30.0;
    let mut means = Vec::new();
    for i in 0..120 {
        let start = i as f64 * t_frame / 1e6 * 10.0;
        let duty = 0.2;
        let frac = (start % 1.0) < duty;
        means.push(if frac { 0.9 } else { 0.3 });
    }
    let g = temporal_gain(&means, 15, 4.0);
    let fixed: Vec<f64> = means.iter().zip(g.iter()).map(|(v, k)| v * k).collect();
    assert!(spread(&fixed) < spread(&means) * 0.4, "means {} -> {}", spread(&means), spread(&fixed));
}

#[test]
fn gain_stays_flat_on_a_clean_clip() {
    let means = vec![0.5; 60];
    let g = temporal_gain(&means, 15, 4.0);
    assert!(g.iter().all(|v| (v - 1.0).abs() < 1e-9));
    let p = vec![0.4; ROWS];
    let (g, f) = row_gain(&p, LINE_RATE, 3.0);
    assert_eq!(f, 0.0, "no flicker should be reported");
    assert!(g.iter().all(|v| (v - 1.0).abs() < 1e-9));
}
