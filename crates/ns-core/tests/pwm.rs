//! Tests for the PWM (LED) mode: frequency from the vertical banding profile,
//! closed loop that lands on an exposure equal to k / f.

use ns_core::camera::CameraLimits;
use ns_core::controller::State;
use ns_core::pwm;
use ns_core::pwm_controller::PwmController;

/// Vertical luminance profile of one frame. Row `r` is read at `r / line_rate_hz`
/// seconds and integrates the square-wave LED over its exposure window.
fn profile_for(exposure_us: f64, f_hz: f64, phase_s: f64, duty: f64, line_rate_hz: f64, rows: usize) -> Vec<f64> {
    ns_core::pwm::synthetic_profile(exposure_us, f_hz, phase_s, duty, line_rate_hz, rows)
}

fn limits() -> CameraLimits {
    CameraLimits {
        min_exposure_us: 100.0,
        max_exposure_us: 30000.0,
        sustain_us: 5000.0,
        supported_fps: vec![30.0],
        min_iso: 100.0,
        max_iso: 6400.0,
    }
}

const LINE_RATE: f64 = 40000.0;
const ROWS: usize = 512;

#[test]
fn fft_finds_500hz_flicker() {
    let p = profile_for(1000.0, 500.0, 0.13, 0.3, LINE_RATE, ROWS);
    let (f, conf) = pwm::estimate_f(&p, LINE_RATE, 100.0, 5000.0).expect("no peak");
    assert!((f - 500.0).abs() < 5.0, "f = {f}");
    assert!(conf > 0.3, "conf = {conf}");
}

#[test]
fn closed_loop_locks_on_integer_multiple() {
    let f_hz = 500.0;
    let mut c = PwmController::new(limits(), LINE_RATE);
    let mut phase = 0.0;
    let mut last = c.on_profile(&profile_for(1000.0, f_hz, phase, 0.3, LINE_RATE, ROWS));
    for _ in 0..200 {
        phase += 0.01;
        let p = profile_for(last.exposure_us, f_hz, phase, 0.3, LINE_RATE, ROWS);
        last = c.on_profile(&p);
        if last.state == State::Locked {
            break;
        }
    }
    assert_eq!(last.state, State::Locked, "never locked");
    let t_us = 1e6 / f_hz;
    let k = (last.exposure_us / t_us).round();
    assert!((last.exposure_us - k * t_us).abs() < 0.02 * t_us, "exposure {} us, k {k}", last.exposure_us);
    let p = profile_for(last.exposure_us, f_hz, phase, 0.3, LINE_RATE, ROWS);
    assert!(pwm::banding_amplitude(&p) < 0.01, "residual banding {}", pwm::banding_amplitude(&p));
}

#[test]
fn relocks_when_flicker_frequency_drifts() {
    let mut c = PwmController::new(limits(), LINE_RATE);
    let mut f_hz = 500.0;
    let mut phase = 0.0;
    let mut last = c.on_profile(&profile_for(1000.0, f_hz, phase, 0.3, LINE_RATE, ROWS));
    // The periodic recalib (every CHECK_FRAMES) spends frames in Search, so the
    // state is not Locked at a fixed frame. Take the last exposure that locked.
    let mut best = None;
    for _ in 0..120 {
        phase += 0.01;
        last = c.on_profile(&profile_for(last.exposure_us, f_hz, phase, 0.3, LINE_RATE, ROWS));
        if last.state == State::Locked {
            best = Some(last.exposure_us);
        }
    }
    let t_us = 1e6 / f_hz;
    let e = best.expect("never locked");
    let k = (e / t_us).round();
    assert!((e - k * t_us).abs() < 0.02 * t_us, "exposure {e} us");
    // The lighting desk changes the PWM rate.
    f_hz = 512.0;
    best = None;
    for _ in 0..200 {
        phase += 0.01;
        last = c.on_profile(&profile_for(last.exposure_us, f_hz, phase, 0.3, LINE_RATE, ROWS));
        if last.state == State::Locked {
            best = Some(last.exposure_us);
        }
    }
    let t_us = 1e6 / f_hz;
    let e = best.expect("did not relock after drift");
    let k = (e / t_us).round();
    assert!((e - k * t_us).abs() < 0.02 * t_us, "exposure {e} us");
}

#[test]
fn sweep_finds_frequency_without_line_rate() {
    let mut c = PwmController::new(limits(), 0.0);
    let f_hz = 500.0;
    let mut phase = 0.0;
    let mut last = c.on_profile(&profile_for(1000.0, f_hz, phase, 0.3, LINE_RATE, ROWS));
    for _ in 0..400 {
        phase += 0.01;
        last = c.on_profile(&profile_for(last.exposure_us, f_hz, phase, 0.3, LINE_RATE, ROWS));
        if last.state == State::Locked {
            break;
        }
    }
    assert_eq!(last.state, State::Locked, "never locked");
    let t_us = 1e6 / f_hz;
    let k = (last.exposure_us / t_us).round();
    assert!((last.exposure_us - k * t_us).abs() < 0.05 * t_us, "exposure {} us", last.exposure_us);
}

#[test]
fn fft_finds_1khz_flicker_within_one_percent() {
    let p = profile_for(100.0, 1000.0, 0.37, 0.4, LINE_RATE, ROWS);
    let (f, conf) = pwm::estimate_f(&p, LINE_RATE, 100.0, 5000.0).expect("no peak");
    assert!((f - 1000.0).abs() < 10.0, "f = {f}");
    assert!(conf > 0.3, "conf = {conf}");
}

#[test]
fn static_scene_never_locks() {
    let mut c = PwmController::new(limits(), LINE_RATE);
    let flat = vec![0.5f64; ROWS];
    for _ in 0..200 {
        let cmd = c.on_profile(&flat);
        assert_eq!(cmd.state, State::Search, "locked on a static scene");
    }
}

/// Deterministic noise in [-amp, amp] (no rand dependency).
fn noise(seed: &mut u64, amp: f64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 33) as f64 / (1u64 << 31) as f64 - 0.5) * 2.0 * amp
}

fn never_locks_on_flat_scene(level: f64, noise_amp: f64) {
    let mut c = PwmController::new(limits(), 0.0);
    let mut seed = 42u64;
    for _ in 0..1500 {
        let p: Vec<f64> = (0..ROWS).map(|_| level + noise(&mut seed, noise_amp)).collect();
        let cmd = c.on_profile(&p);
        assert_ne!(cmd.state, State::Locked, "locked on a scene without flicker (f = {} Hz)", c.frequency_hz());
    }
    assert_eq!(c.frequency_hz(), 0.0, "invented a frequency from noise");
}

#[test]
fn bright_scene_without_flicker_never_locks_without_line_rate() {
    never_locks_on_flat_scene(0.5, 0.002);
}

#[test]
fn dark_noisy_scene_never_locks_without_line_rate() {
    // What the Pixel sees when pointed at a dark room: tiny luma, noise-dominated.
    never_locks_on_flat_scene(0.01, 0.003);
}
