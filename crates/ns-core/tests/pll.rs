use ns_core::pll::{FlickerPll, PllConfig, measure_profile_phase};
use std::f64::consts::PI;

#[test]
fn test_pll_phase_and_frequency_tracking() {
    let mut pll = FlickerPll::new(PllConfig {
        kp: 0.35,
        ki: 0.15,
        pipeline_depth: 2.0,
    });

    let target_f = 500.0; // 500 Hz flicker
    let frame_dt_us = 1e6 / 30.0; // 30 fps (33,333.3 us per frame)
    let initial_phase = 0.5 * PI;

    // Initialize with a slightly off frequency estimate
    pll.init(490.0, initial_phase, 0.0);

    for frame in 1..=60 {
        let t_us = frame as f64 * frame_dt_us;
        // Ground truth phase at sensor
        let true_phase = (initial_phase + 2.0 * PI * target_f * (t_us * 1e-6)).rem_euclid(2.0 * PI);
        // Measurement with small Gaussian-like noise
        let noise = 0.05 * ((frame as f64 * 1.7).sin());
        let meas_phase = (true_phase + noise).rem_euclid(2.0 * PI);

        pll.update(meas_phase, t_us, 1.0);
    }

    // After 60 frames (2 seconds), PLL should accurately track 500 Hz
    assert!((pll.frequency_hz() - target_f).abs() < 1.0, "Frequency tracking error: {}", pll.frequency_hz());
    assert!(pll.confidence() > 0.7, "Confidence too low: {}", pll.confidence());
}

#[test]
fn test_pll_pipeline_depth_lead_prediction() {
    let mut pll = FlickerPll::new(PllConfig {
        kp: 0.25,
        ki: 0.02,
        pipeline_depth: 2.5, // 2.5 frames forward prediction
    });

    let f = 200.0; // 200 Hz
    let frame_us = 1e6 / 60.0; // 60 fps
    pll.init(f, 0.0, 0.0);

    // Predict 2.5 frames ahead
    let predicted_phase = pll.predict_lead_phase(frame_us);

    // Ground truth phase 2.5 frames ahead: dt = 2.5 * frame_us
    let dt_s = 2.5 * frame_us * 1e-6;
    let expected_phase = (2.0 * PI * f * dt_s).rem_euclid(2.0 * PI);

    let diff = (predicted_phase - expected_phase).abs();
    assert!(diff < 1e-6 || (diff - 2.0 * PI).abs() < 1e-6, "Prediction mismatch: pred={}, exp={}", predicted_phase, expected_phase);
}

#[test]
fn test_measure_profile_phase_synthetic() {
    let period_rows = 80.0; // 80 rows per cycle
    let rows = 400;
    let known_phase = 1.234;

    let profile: Vec<f64> = (0..rows)
        .map(|y| {
            let theta = 2.0 * PI * (y as f64) / period_rows + known_phase;
            0.5 + 0.3 * theta.sin()
        })
        .collect();

    let measured = measure_profile_phase(&profile, period_rows).expect("measured phase");
    let diff = (measured - known_phase).rem_euclid(2.0 * PI);
    let err = diff.min(2.0 * PI - diff);
    assert!(err < 0.05, "Phase measurement error: {err} rad");
}
