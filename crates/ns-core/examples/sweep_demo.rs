//! Trace the sweep calibration (no row readout rate) on a synthetic LED flicker.

use ns_core::camera::CameraLimits;
use ns_core::pwm;
use ns_core::pwm_controller::PwmController;

fn profile_for(exposure_us: f64, f_hz: f64, phase_s: f64, duty: f64, line_rate_hz: f64, rows: usize) -> Vec<f64> {
    ns_core::pwm::synthetic_profile(exposure_us, f_hz, phase_s, duty, line_rate_hz, rows)
}

fn main() {
    let f_hz = 500.0;
    let line_rate = 40000.0;
    let rows = 512usize;
    let limits = CameraLimits {
        min_exposure_us: 100.0,
        max_exposure_us: 30000.0,
        sustain_us: 5000.0,
        supported_fps: vec![30.0],
        min_iso: 100.0,
        max_iso: 6400.0,
    };
    let mut c = PwmController::new(limits, 0.0);
    let mut phase = 0.0;
    let mut exposure = 100.0;
    for frame in 0..400 {
        let p = profile_for(exposure, f_hz, phase, 0.3, line_rate, rows);
        let band = pwm::banding_amplitude(&p);
        let cmd = c.on_profile(&p);
        exposure = cmd.exposure_us;
        phase += 0.01;
        if frame % 6 == 0 {
            println!(
                "frame {frame:3}  {:6}  exposure {:8.1}  band {band:.4}  period {:8.1}",
                format!("{:?}", cmd.state),
                cmd.exposure_us,
                cmd.period_us
            );
        }
    }
}
