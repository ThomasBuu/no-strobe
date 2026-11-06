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

/// Limits a real phone reports (Xperia 1 III): minimum ~0, long maximum, 30 fps only.
fn phone_limits() -> CameraLimits {
    CameraLimits {
        min_exposure_us: 20.0,
        max_exposure_us: 157_076.0,
        // The 1080p readout of the Xperia: 1080 rows at 40 kHz = 27 ms. The profile here has 512
        // rows read at 40 kHz, so the readout is 12,8 ms and one row is 25 us.
        sustain_us: 12_800.0,
        supported_fps: vec![30.0],
        min_iso: 75.0,
        max_iso: 9618.0,
    }
}

/// The high-speed surface list the Xperia reports for 1080p: up to 240 fps.
fn high_speed_limits() -> CameraLimits {
    CameraLimits {
        min_exposure_us: 20.0,
        max_exposure_us: 157_076.0,
        sustain_us: 0.0,
        supported_fps: vec![30.0, 60.0, 120.0, 240.0],
        min_iso: 75.0,
        max_iso: 9618.0,
    }
}

#[test]
fn highest_frame_rate_that_fits_the_period_is_chosen() {
    // Paul's rule for high-speed recording: fps <= f, k = 1. At 500 Hz the period is 2 ms and a
    // 240 fps frame is 4,17 ms, so 240 fits; at 200 Hz the period is 5 ms and 240 fps cannot
    // hold one period plus the readout, so the climb must drop to 120 fps, not stay at 240.
    for (f_hz, want_fps) in [(500.0, 240.0), (200.0, 120.0), (100.0, 60.0)] {
        let mut c = PwmController::new(high_speed_limits(), 0.0);
        let mut phase = 0.0;
        // The rows of a frame are read across the frame, so the row rate is rows x fps. The
        // controller measures the banding period in rows and converts it with the frame time, so
        // the profile has to be read at the frame rate the controller commanded.
        let mut last = c.on_profile(&profile_for(1000.0, f_hz, phase, 0.3, ROWS as f64 * 240.0, ROWS));
        let mut got = None;
        for _ in 0..900 {
            phase += 0.3337;
            let line_rate = ROWS as f64 * last.fps;
            last = c.on_profile(&profile_for(last.exposure_us, f_hz, phase, 0.3, line_rate, ROWS));
            if last.state == State::Locked {
                got = Some(last);
                break;
            }
        }
        let cmd = got.unwrap_or_else(|| panic!("never locked at {f_hz} Hz"));
        let t_us = 1e6 / f_hz;
        let k = (cmd.exposure_us / t_us).round();
        assert_eq!(cmd.fps, want_fps, "{f_hz} Hz picked {} fps", cmd.fps);
        assert!(
            cmd.exposure_us + 1e6 / cmd.fps >= cmd.exposure_us,
            "frame must hold the exposure"
        );
        assert!((cmd.exposure_us - k * t_us).abs() < 0.02 * t_us, "{f_hz} Hz: {} us is not k/f", cmd.exposure_us);
        assert!(1e6 / cmd.fps >= cmd.exposure_us, "exposure longer than the frame at {f_hz} Hz");
    }
}

#[test]
fn blind_sweep_with_phone_limits_locks_on_a_multiple_of_the_period() {
    for f_hz in [300.0, 500.0, 1000.0, 2000.0] {
        let mut c = PwmController::new(phone_limits(), 0.0);
        let mut phase = 0.0;
        let mut last = c.on_profile(&profile_for(1000.0, f_hz, phase, 0.3, LINE_RATE, ROWS));
        for _ in 0..600 {
            phase += 0.3337;
            last = c.on_profile(&profile_for(last.exposure_us, f_hz, phase, 0.3, LINE_RATE, ROWS));
            if last.state == State::Locked {
                break;
            }
        }
        assert_eq!(last.state, State::Locked, "never locked at {f_hz} Hz");
        let t_us = 1e6 / f_hz;
        let k = (last.exposure_us / t_us).round();
        assert!(k >= 1.0, "locked below one period at {f_hz} Hz: {} us", last.exposure_us);
        assert!(
            (last.exposure_us - k * t_us).abs() < 0.03 * t_us,
            "{f_hz} Hz: exposure {} us is not a multiple of {t_us} us",
            last.exposure_us
        );
    }
}

#[test]
fn blind_lock_holds_over_time_and_follows_a_frequency_change() {
    let mut c = PwmController::new(phone_limits(), 0.0);
    let mut f_hz = 500.0;
    let mut phase = 0.0;
    let mut last = c.on_profile(&profile_for(1000.0, f_hz, phase, 0.3, LINE_RATE, ROWS));
    let mut locked_at = None;
    for i in 0..600 {
        phase += 0.3337;
        last = c.on_profile(&profile_for(last.exposure_us, f_hz, phase, 0.3, LINE_RATE, ROWS));
        if last.state == State::Locked && locked_at.is_none() {
            locked_at = Some(i);
        }
    }
    let start = locked_at.expect("never locked");
    // Hold: 600 more frames at the same flicker stay locked (the periodic re-check is local).
    let mut locked_frames = 0;
    let mut min_ratio = f64::INFINITY;
    let mut max_ratio = 0.0f64;
    for _ in 0..600 {
        phase += 0.3337;
        last = c.on_profile(&profile_for(last.exposure_us, f_hz, phase, 0.3, LINE_RATE, ROWS));
        if last.state == State::Locked {
            locked_frames += 1;
        }
        let r = last.exposure_us / (1e6 / f_hz);
        min_ratio = min_ratio.min(r);
        max_ratio = max_ratio.max(r);
    }
    assert!(locked_frames >= 540, "locked on only {locked_frames}/600 frames after locking at {start}");
    let k = ((min_ratio + max_ratio) / 2.0).round();
    assert!(min_ratio > k - 0.06 && max_ratio < k + 0.06, "exposure wandered: {min_ratio}..{max_ratio} periods");

    // The light changes frequency: the lock is lost and found again at the new multiple.
    f_hz = 640.0;
    let mut relocked = false;
    for _ in 0..900 {
        phase += 0.3337;
        last = c.on_profile(&profile_for(last.exposure_us, f_hz, phase, 0.3, LINE_RATE, ROWS));
        if last.state == State::Locked {
            let t_us = 1e6 / f_hz;
            let k = (last.exposure_us / t_us).round();
            if k >= 1.0 && (last.exposure_us - k * t_us).abs() < 0.03 * t_us {
                relocked = true;
                break;
            }
        }
    }
    assert!(relocked, "did not relock after 500 -> 640 Hz");
}

#[test]
fn locked_exposure_never_moves_in_blind_mode() {
    // A recording cannot afford exposure steps: every probe of a re-check changes the exposure the
    // sensor uses, so the clip shows a brightness step. Once the null is found, the exposure stays.
    let mut c = PwmController::new(phone_limits(), 0.0);
    let f_hz = 500.0;
    let mut phase = 0.0;
    let mut last = c.on_profile(&profile_for(1000.0, f_hz, phase, 0.3, LINE_RATE, ROWS));
    let mut locked_at = None;
    for i in 0..600 {
        phase += 0.3337;
        last = c.on_profile(&profile_for(last.exposure_us, f_hz, phase, 0.3, LINE_RATE, ROWS));
        if last.state == State::Locked {
            locked_at = Some(i);
            break;
        }
    }
    let start = locked_at.expect("never locked");
    let fixed = last.exposure_us;
    let t_us = 1e6 / f_hz;
    let k = (fixed / t_us).round();
    assert!((fixed - k * t_us).abs() < 0.01 * t_us, "locked off the null: {fixed} us");
    for i in 0..300 {
        phase += 0.3337;
        last = c.on_profile(&profile_for(last.exposure_us, f_hz, phase, 0.3, LINE_RATE, ROWS));
        assert_eq!(last.state, State::Locked, "left the lock at frame {i} after locking at {start}");
        assert_eq!(last.exposure_us, fixed, "exposure moved to {} us at frame {i}", last.exposure_us);
    }
}

#[test]
fn noisy_blind_sweep_never_locks_on_a_false_minimum() {
    // A real sensor adds noise to every row: a "minimum" found in noise must not count. The level is
    // the measured one: temporal noise of a real scene at ISO ~3000 is 0.004-0.009 of the mean luma.
    for (f_hz, seed0) in [(500.0, 1u64), (500.0, 7), (800.0, 3), (1200.0, 11), (300.0, 5)] {
        let mut c = PwmController::new(phone_limits(), 0.0);
        let mut seed = seed0;
        let mut phase = 0.0;
        let noisy = |e: f64, phase: f64, seed: &mut u64| -> Vec<f64> {
            profile_for(e, f_hz, phase, 0.3, LINE_RATE, ROWS)
                .into_iter()
                .map(|v| v + noise(seed, 0.006))
                .collect()
        };
        let mut last = c.on_profile(&noisy(1000.0, phase, &mut seed));
        for _ in 0..1500 {
            phase += 0.3337;
            last = c.on_profile(&noisy(last.exposure_us, phase, &mut seed));
            if last.state == State::Locked {
                // What matters is the banding left at the locked exposure, against the banding
                // an unsynchronised exposure (half a period) would show.
                let clean = |e: f64| pwm::banding_amplitude(&profile_for(e, f_hz, 0.31, 0.3, LINE_RATE, ROWS));
                let left = clean(last.exposure_us);
                let unsynced = clean(0.5e6 / f_hz) * (0.5e6 / f_hz) / last.exposure_us.max(1.0);
                assert!(
                    last.exposure_us >= 1e6 / 5000.0 && left < 0.3 * unsynced.max(0.01),
                    "{f_hz} Hz (seed {seed0}): locked at {} us leaving banding {left:.4} (f = {} Hz)",
                    last.exposure_us,
                    c.frequency_hz()
                );
            }
        }
    }
}

#[test]
fn locks_on_a_long_multiple_of_the_period_when_a_shutter_fraction_is_set() {
    // 500 Hz at 30 fps: a 180 degree shutter is 16.7 ms, i.e. 8 periods of 2 ms.
    let f_hz = 500.0;
    let mut c = PwmController::new(phone_limits(), 0.0);
    c.set_shutter_fraction(0.5);
    let mut phase = 0.0;
    let mut last = c.on_profile(&profile_for(1000.0, f_hz, phase, 0.3, LINE_RATE, ROWS));
    for _ in 0..1500 {
        phase += 0.3337;
        last = c.on_profile(&profile_for(last.exposure_us, f_hz, phase, 0.3, LINE_RATE, ROWS));
    }
    assert_eq!(last.state, State::Locked);
    let t_us = 1e6 / f_hz;
    let k = (last.exposure_us / t_us).round();
    assert!(k >= 6.0, "stayed on a short exposure: {} us (k = {k})", last.exposure_us);
    assert!((last.exposure_us - k * t_us).abs() < 0.02 * t_us, "{} us is not a multiple of {t_us}", last.exposure_us);
    assert!(last.exposure_us <= 1e6 / 30.0, "longer than a frame: {} us", last.exposure_us);
}

/// What a real lamp showed: the scene structure along the rows is several times larger than the
/// flicker (a bright lamp, cables, a wall edge), static from frame to frame. The spatial banding
/// metric stayed flat (0.022 at every exposure from 4 ms) while the per-row variation between
/// frames had clean nulls at 10, 20 and 30 ms.
#[test]
fn locks_on_a_multiple_of_the_period_in_a_scene_full_of_structure() {
    for f_hz in [100.0, 300.0, 500.0, 1000.0] {
        let mut c = PwmController::new(phone_limits(), 0.0);
        // Static structure, ~3x the flicker, identical in every frame.
        let scene: Vec<f64> = (0..ROWS)
            .map(|i| {
                let x = i as f64;
                0.55 + 0.30 * (2.0 * std::f64::consts::PI * x / 97.0).sin()
                    + 0.20 * (2.0 * std::f64::consts::PI * x / 41.0 + 1.0).sin()
                    + if (180..260).contains(&i) { 0.9 } else { 0.0 }
            })
            .collect();
        let frame = |e: f64, phase: f64| -> Vec<f64> {
            profile_for(e, f_hz, phase, 0.3, LINE_RATE, ROWS)
                .iter()
                .zip(&scene)
                .map(|(fl, s)| s * (0.75 + 0.25 * fl))
                .collect()
        };
        let mut phase = 0.0;
        let mut last = c.on_profile(&frame(1000.0, phase));
        for _ in 0..2500 {
            phase += 0.3337;
            last = c.on_profile(&frame(last.exposure_us, phase));
            if std::env::var("TRACE").is_ok() && (phase * 3.0) as u64 % 60 == 0 {
                eprintln!("{:.0} Hz  e={:.0} us {:?} f={:.1}", f_hz, last.exposure_us, last.state, c.frequency_hz());
            }
            if last.state == State::Locked {
                break;
            }
        }
        assert_eq!(last.state, State::Locked, "never locked at {f_hz} Hz in a structured scene");
        let t_us = 1e6 / f_hz;
        let k = (last.exposure_us / t_us).round();
        assert!(k >= 1.0, "{f_hz} Hz: locked below one period: {} us", last.exposure_us);
        assert!(
            (last.exposure_us - k * t_us).abs() < 0.06 * t_us,
            "{f_hz} Hz: {} us is not a multiple of {t_us} us",
            last.exposure_us
        );
    }
}
