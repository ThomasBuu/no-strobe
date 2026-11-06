//! Trace the relock path when the PWM rate changes mid-clip.

use ns_core::camera::CameraLimits;
use ns_core::pwm::synthetic_profile;
use ns_core::pwm_controller::PwmController;

fn main() {
    let limits = CameraLimits {
        min_exposure_us: 100.0,
        max_exposure_us: 30000.0,
        sustain_us: 5000.0,
        supported_fps: vec![30.0],
        min_iso: 100.0,
        max_iso: 6400.0,
    };
    let line = 40000.0;
    let mut c = PwmController::new(limits, line);
    let mut f_hz = 500.0;
    let mut phase = 0.0;
    let mut last = c.on_profile(&synthetic_profile(1000.0, f_hz, phase, 0.3, line, 512));
    for frame in 0..320 {
        phase += 0.01;
        if frame == 120 {
            f_hz = 512.0;
            println!("--- f -> 512 Hz");
        }
        last = c.on_profile(&synthetic_profile(last.exposure_us, f_hz, phase, 0.3, line, 512));
        if frame % 10 == 0 {
            println!(
                "frame {frame:3}  {:?}  e {:>9.1}  f {:>7.1}  conf {:.2}",
                last.state, last.exposure_us, last.period_us, last.confidence
            );
        }
    }
}
