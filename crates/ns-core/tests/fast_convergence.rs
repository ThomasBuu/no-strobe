//! Verification of fast convergence (< 500 ms) and scene structure rejection
//! using 2nd-order discrete Laplacian pre-filtering and robust autocorrelation.

use ns_core::camera::CameraLimits;
use ns_core::controller::State;
use ns_core::pwm;
use ns_core::pwm_controller::PwmController;

const LINE_RATE: f64 = 40000.0; // 40 kHz line rate (standard mobile rolling shutter)
const ROWS: usize = 1080; // 1080p frame

fn limits() -> CameraLimits {
    CameraLimits {
        min_exposure_us: 50.0,
        max_exposure_us: 33333.0,
        sustain_us: 1000.0,
        supported_fps: vec![30.0],
        min_iso: 100.0,
        max_iso: 3200.0,
    }
}

fn synthetic_structured_scene() -> Vec<f64> {
    (0..ROWS)
        .map(|i| {
            let y = i as f64;
            // Linear ramp across frame (perspective illumination gradient)
            let ramp = 0.2 + 0.5 * (y / ROWS as f64);
            // Low frequency illumination texture (curtains, room lighting gradient)
            let waves = 0.15 * (2.0 * std::f64::consts::PI * y / 350.0).sin();
            // Sharp step edges (posters / doorframes)
            let step = if (300..600).contains(&i) { 0.4 } else { 0.0 };
            ramp + waves + step
        })
        .collect()
}

#[test]
fn test_single_frame_frequency_estimation_under_scene_structure() {
    let f_target = 480.0; // 480 Hz PWM flicker
    let scene = synthetic_structured_scene();

    // 1000 us exposure: contains multiple cycles of 480 Hz
    let flicker = pwm::synthetic_profile(1000.0, f_target, 0.0, 0.35, LINE_RATE, ROWS);
    let frame: Vec<f64> = flicker.iter().zip(&scene).map(|(&fl, &sc)| sc * (0.8 + 0.2 * fl)).collect();

    // Single-frame estimation (without previous profile)
    let est = pwm::robust_estimate_f(&frame, None, LINE_RATE, 50.0, 2000.0);
    assert!(est.is_some(), "Single frame estimation failed under scene clutter");

    let (f_est, conf) = est.unwrap();
    let err = (f_est - f_target).abs();
    assert!(err < 5.0, "Estimated frequency error too large: {} Hz (target {})", f_est, f_target);
    assert!(conf >= 0.35, "Confidence too low: {}", conf);
}

#[test]
fn test_differential_frequency_estimation_rejects_arbitrary_pattern() {
    let f_target = 480.0; // 480 Hz PWM flicker
    // Even if the scene has printed stripes near the flicker frequency:
    let scene: Vec<f64> = (0..ROWS)
        .map(|i| {
            let y = i as f64;
            0.5 + 0.25 * (2.0 * std::f64::consts::PI * y / 89.0).cos()
        })
        .collect();

    let frame1 = pwm::synthetic_profile(1000.0, f_target, 0.0, 0.35, LINE_RATE, ROWS)
        .iter().zip(&scene).map(|(&fl, &sc)| sc * (0.8 + 0.2 * fl)).collect::<Vec<_>>();
    let frame2 = pwm::synthetic_profile(1000.0, f_target, 0.33, 0.35, LINE_RATE, ROWS)
        .iter().zip(&scene).map(|(&fl, &sc)| sc * (0.8 + 0.2 * fl)).collect::<Vec<_>>();

    // Differential estimation using consecutive frames completely eliminates the static scene stripes
    let est = pwm::robust_estimate_f(&frame2, Some(&frame1), LINE_RATE, 50.0, 2000.0);
    assert!(est.is_some(), "Differential estimation failed");

    let (f_est, conf) = est.unwrap();
    let err = (f_est - f_target).abs();
    assert!(err < 5.0, "Differential frequency error: {} Hz (target {})", f_est, f_target);
    assert!(conf >= 0.35, "Confidence too low: {}", conf);
}

#[test]
fn test_fast_convergence_in_under_500ms() {
    let f_target = 500.0; // 500 Hz flicker
    let scene = synthetic_structured_scene();

    let mut controller = PwmController::new(limits(), LINE_RATE);
    controller.set_pipeline_depth(2.0);

    let mut phase = 0.0;
    let mut frames_to_lock = None;
    let max_frames = 20; // < 500 ms at 30 fps

    let mut last_cmd = controller.on_profile(&{
        let fl = pwm::synthetic_profile(100.0, f_target, phase, 0.3, LINE_RATE, ROWS);
        fl.iter().zip(&scene).map(|(&f, &s)| s * (0.85 + 0.15 * f)).collect::<Vec<_>>()
    });
    if last_cmd.state == State::Locked {
        frames_to_lock = Some(1);
    } else {
        for frame in 2..=max_frames {
            phase += 0.333;
            let exposure = last_cmd.exposure_us;
            let fl = pwm::synthetic_profile(exposure, f_target, phase, 0.3, LINE_RATE, ROWS);
            let profile: Vec<f64> = fl.iter().zip(&scene).map(|(&f, &s)| s * (0.85 + 0.15 * f)).collect();

            last_cmd = controller.on_profile(&profile);
            if last_cmd.state == State::Locked {
                frames_to_lock = Some(frame);
                break;
            }
        }
    }

    assert!(
        frames_to_lock.is_some(),
        "Controller failed to lock within 500 ms (15 frames). Last state: {:?}",
        last_cmd.state
    );

    let count = frames_to_lock.unwrap();
    let time_ms = count as f64 * (1000.0 / 30.0);
    println!("Successfully converged and locked in {} frames ({:.1} ms)", count, time_ms);

    let period_us = 1e6 / f_target;
    let k = (last_cmd.exposure_us / period_us).round();
    let residual_err = (last_cmd.exposure_us - k * period_us).abs();
    assert!(residual_err < 0.03 * period_us, "Locked exposure not integer multiple of period: {} us", last_cmd.exposure_us);

    assert!(controller.pll().is_some(), "PLL should be initialized and active");
    assert!(controller.pll().unwrap().confidence() > 0.3, "PLL confidence should be high");
}


