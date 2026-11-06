use ns_core::camera::CameraLimits;
use ns_core::controller::{Controller, State};
use ns_core::solver::Mode;

/// Synthetic strobe: exposure window [t - exposure, t] overlapping a pulse train.
fn synth(t_us: f64, exposure_us: f64, period_us: f64, phase_us: f64, flash_us: f64) -> f64 {
    let start = t_us - exposure_us;
    let mut lit = 0.0;
    let mut cycle = ((start - phase_us) / period_us).floor();
    while cycle * period_us + phase_us < t_us {
        let pulse_start = cycle * period_us + phase_us;
        let pulse_end = pulse_start + flash_us;
        let overlap = (t_us.min(pulse_end) - start.max(pulse_start)).max(0.0);
        lit += overlap;
        cycle += 1.0;
    }
    0.2 + 0.8 * lit / exposure_us
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Integrate => "integrate",
        Mode::Gap => "gap",
        Mode::Flash => "flash",
        Mode::Pwm => "pwm",
    }
}

fn main() {
    let period_us = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000.0);
    // Search runs at the fps the controller commands. A 30 fps feed cannot show a 50 Hz
    // strobe to frame-mean luma: 50 Hz is above the 30 fps Nyquist limit.
    let fps = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(30.0);
    let frame_us = 1e6 / fps;

    let mut controller = Controller::new(CameraLimits::typical_phone());

    println!("strobe period {:.0} us ({:.1} Hz), camera at {:.0} fps",
        period_us, 1e6 / period_us, fps);

    for i in 0..150 {
        let t_us = i as f64 * frame_us;
        // the app runs at the short search exposure until the controller locks
        let cmd = controller.on_frame(t_us, synth(t_us, 8_000.0, period_us, 30_000.0, 10_000.0));
        if i % 10 == 0 || cmd.state == State::Locked {
            println!(
                "frame {:>3}  {:>6}  exposure {:>8.0} us  fps {:>2.0}  iso {:>4.0}  T {:>8.0}  conf {:.2}  drift {:>6.0}",
                i,
                format!("{:?}", cmd.state),
                cmd.exposure_us,
                cmd.fps,
                cmd.iso,
                cmd.period_us,
                cmd.confidence,
                cmd.drift_us_per_frame,
            );
            if cmd.state == State::Locked {
                println!("           mode: {}", mode_name(cmd.mode));
            }
        }
    }
}
