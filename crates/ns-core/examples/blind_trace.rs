//! Trace of the blind (no row rate) closed loop on a synthetic PWM light:
//!     cargo run -p ns-core --example blind_trace [f_hz] [frames]
use ns_core::camera::CameraLimits;
use ns_core::pwm::synthetic_profile;
use ns_core::pwm_controller::PwmController;

fn main() {
    let f_hz: f64 = std::env::args().nth(1).and_then(|v| v.parse().ok()).unwrap_or(500.0);
    let frames: usize = std::env::args().nth(2).and_then(|v| v.parse().ok()).unwrap_or(400);
    let limits = CameraLimits {
        min_exposure_us: 20.0,
        max_exposure_us: 157_076.0,
        sustain_us: 0.0,
        supported_fps: vec![30.0],
        min_iso: 75.0,
        max_iso: 9618.0,
    };
    let mut c = PwmController::new(limits, 0.0);
    let mut phase = 0.0;
    let mut e = 1000.0;
    for i in 0..frames {
        phase += 0.0137;
        let cmd = c.on_profile(&synthetic_profile(e, f_hz, phase, 0.3, 40_000.0, 512));
        e = cmd.exposure_us;
        if i % 4 == 0 {
            println!("{i:4} {:>7.1} us  {:?}  f={:.1}", e, cmd.state, c.frequency_hz());
        }
    }
}
