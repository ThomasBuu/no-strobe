//! Trace the PWM closed loop on a synthetic LED flicker.
//!
//! Run: `cargo run -p ns-core --example pwm_demo [f_hz] [line_rate]`

use ns_core::camera::CameraLimits;
use ns_core::pwm;
use ns_core::pwm_controller::PwmController;

fn profile_for(exposure_us: f64, f_hz: f64, phase_s: f64, duty: f64, line_rate_hz: f64, rows: usize) -> Vec<f64> {
    ns_core::pwm::synthetic_profile(exposure_us, f_hz, phase_s, duty, line_rate_hz, rows)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let f_hz: f64 = args.get(1).map(|s| s.parse().unwrap()).unwrap_or(500.0);
    let line_rate: f64 = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(40000.0);
    let rows = 512usize;
    let limits = CameraLimits {
        min_exposure_us: 100.0,
        max_exposure_us: 30000.0,
        sustain_us: 5000.0,
        supported_fps: vec![30.0],
        min_iso: 100.0,
        max_iso: 6400.0,
    };
    let mut c = PwmController::new(limits, line_rate);
    let mut phase = 0.0;
    let mut exposure = 1000.0;
    for frame in 0..200 {
        let p = profile_for(exposure, f_hz, phase, 0.3, line_rate, rows);
        let band = pwm::banding_amplitude(&p);
        let cmd = c.on_profile(&p);
        exposure = cmd.exposure_us;
        phase += 0.01;
        println!(
            "frame {frame:3}  {:6}  exposure {:8.1}  band {band:.4}  f {:7.1}  conf {:.2}",
            format!("{:?}", cmd.state),
            cmd.exposure_us,
            cmd.period_us,
            cmd.confidence
        );
    }
}
