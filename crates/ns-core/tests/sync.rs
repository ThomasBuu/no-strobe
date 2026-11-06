use ns_core::camera::CameraLimits;
use ns_core::controller::{Controller, State};
use ns_core::detector::Estimate;
use ns_core::solver::{solve, Mode};

/// Luminance produced by a strobe: light is `flash_us` wide, repeating every `period_us`.
/// The exposure window is `[t - exposure_us, t]`, so the frame integrates the overlap.
fn synth(t_us: f64, exposure_us: f64, period_us: f64, phase_us: f64, flash_us: f64) -> f64 {
    let start = t_us - exposure_us;
    let end = t_us;

    let mut overlap = 0.0;
    let m0 = ((start - phase_us) / period_us).floor();
    let mut m = m0;
    while phase_us + m * period_us <= end {
        let f_start = phase_us + m * period_us;
        let f_end = f_start + flash_us;
        overlap += (end.min(f_end) - start.max(f_start)).max(0.0);
        m += 1.0;
    }

    0.2 + 0.8 * (overlap / exposure_us)
}

#[test]
fn detects_10hz_strobe() {
    let fps = 30.0;
    let frame_us = 1e6 / fps;
    let period = 100_000.0;

    let mut c = Controller::new(CameraLimits::typical_phone());
    let mut t = 0.0;
    let mut last = None;

    for _ in 0..120 {
        let lum = synth(t, 8_000.0, period, 30_000.0, 10_000.0);
        last = Some(c.on_frame(t, lum));
        t += frame_us;
    }

    let cmd = last.unwrap();
    assert_eq!(cmd.state, State::Locked, "controller did not lock");
    let err = (cmd.period_us - period).abs() / period;
    assert!(err < 0.02, "period error {} (got {}, want {})", err, cmd.period_us, period);
}

#[test]
fn exposure_is_integer_multiple_of_strobe_period() {
    let est = Estimate {
        period_us: 20_000.0,
        phase_us: 0.0,
        duty: 0.1,
        confidence: 0.9,
    };
    let limits = CameraLimits::typical_phone();
    let candidates = solve(&est, &limits);

    assert!(!candidates.is_empty(), "no candidate found");
    let best = candidates[0];
    assert_eq!(best.mode, Mode::Integrate);

    let k = best.exposure_us / est.period_us;
    assert!(
        (k - k.round()).abs() < 1e-6,
        "exposure {} us is not a whole number of strobe cycles (k = {})",
        best.exposure_us,
        k
    );
}

#[test]
fn fast_strobe_falls_back_to_gap_or_flash() {
    // 20 Hz strobe: 50 ms period, longer than any exposure a phone can set at 30 fps,
    // so integrating whole cycles is impossible.
    let est = Estimate {
        period_us: 50_000.0,
        phase_us: 0.0,
        duty: 0.1,
        confidence: 0.9,
    };
    let limits = CameraLimits::typical_phone();
    let candidates = solve(&est, &limits);

    assert!(!candidates.is_empty());
    for c in &candidates {
        assert_ne!(c.mode, Mode::Integrate, "integrate should be infeasible at T = 50 ms");
        assert!(
            c.exposure_us + limits.sustain_us <= 1e6 / c.fps,
            "window {} us does not fit in a {} fps frame",
            c.exposure_us + limits.sustain_us,
            c.fps
        );
    }
}

#[test]
fn window_never_exceeds_frame_period() {
    for period_hz in [5.0, 8.0, 12.0, 20.0, 30.0] {
        let est = Estimate {
            period_us: 1e6 / period_hz,
            phase_us: 0.0,
            duty: 0.1,
            confidence: 0.9,
        };
        let limits = CameraLimits::typical_phone();
        for c in solve(&est, &limits) {
            let frame_us = 1e6 / c.fps;
            assert!(
                c.exposure_us + limits.sustain_us <= frame_us + 1e-9,
                "T = {} Hz, mode {:?}: window {} us > frame {} us",
                period_hz,
                c.mode,
                c.exposure_us + limits.sustain_us,
                frame_us
            );
        }
    }
}
