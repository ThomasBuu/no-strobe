use crate::camera::CameraLimits;
use crate::detector::Estimate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Exposure is an integer number of strobe cycles: every frame integrates the same light.
    Integrate,
    /// Exposure window sits in the dark interval between flashes: no strobe light at all.
    Gap,
    /// Exposure window covers exactly one flash: every frame gets the same flash.
    Flash,
    /// Exposure = k / f for a high-frequency PWM light, found from the vertical
    /// banding profile.
    Pwm,
}

#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    pub mode: Mode,
    pub exposure_us: f64,
    pub fps: f64,
    pub score: f64,
    /// How far the frame period is from the nearest multiple of the strobe period.
    /// This is the phase drift per frame; the window slides through the strobe cycle at this rate.
    pub drift_us_per_frame: f64,
}

pub fn solve(est: &Estimate, limits: &CameraLimits) -> Vec<Candidate> {
    let mut out = Vec::new();
    let t = est.period_us;
    if t <= 0.0 {
        return out;
    }
    let duty = est.duty.clamp(0.02, 0.5);

    for &fps in &limits.supported_fps {
        let frame_us = 1e6 / fps;
        let drift = frame_us - (frame_us / t).round() * t;

        // Integrate: exposure = k full strobe cycles.
        let k = (limits.max_exposure_us / t).floor();
        if k >= 1.0 {
            let e = k * t;
            if e + limits.sustain_us <= frame_us {
                out.push(Candidate {
                    mode: Mode::Integrate,
                    exposure_us: e,
                    fps,
                    score: 0.0,
                    drift_us_per_frame: drift,
                });
            }
        }

        // Gap: window inside the dark interval between flashes.
        let dark = t * (1.0 - duty);
        let e_gap = (dark - limits.sustain_us).min(limits.max_exposure_us);
        if e_gap >= limits.min_exposure_us && e_gap + limits.sustain_us <= frame_us {
            out.push(Candidate {
                mode: Mode::Gap,
                exposure_us: e_gap,
                fps,
                score: 0.0,
                drift_us_per_frame: drift,
            });
        }

        // Flash: window covers one flash.
        let e_flash = (t * duty).min(limits.max_exposure_us);
        if e_flash >= limits.min_exposure_us && e_flash + limits.sustain_us <= frame_us {
            out.push(Candidate {
                mode: Mode::Flash,
                exposure_us: e_flash,
                fps,
                score: 0.0,
                drift_us_per_frame: drift,
            });
        }
    }

    for c in &mut out {
        c.score = score(c, t, duty, limits);
    }
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    out
}

fn score(c: &Candidate, t: f64, duty: f64, limits: &CameraLimits) -> f64 {
    // Rolling shutter exposes rows over exposure + sustain, so the light state a row sees
    // depends on its row index. Banding amplitude grows with window / strobe period.
    let window = c.exposure_us + limits.sustain_us;

    let uniformity = match c.mode {
        Mode::Integrate => 1.0,
        Mode::Gap => {
            if window <= t * (1.0 - duty) {
                1.0
            } else {
                0.5
            }
        }
        Mode::Flash => {
            if window <= t * duty {
                1.0
            } else {
                0.4
            }
        }
        // The PWM controller picks its own exposure; solve() never emits a Pwm
        // candidate, so this arm is only for exhaustiveness.
        Mode::Pwm => 1.0,
    };

    let band_score = 1.0 / (window / t).max(1.0);
    let light = (c.exposure_us / limits.max_exposure_us).clamp(0.0, 1.0);
    let drift_score = 1.0 - (c.drift_us_per_frame.abs() / t).clamp(0.0, 1.0);

    0.45 * uniformity + 0.25 * band_score + 0.20 * light + 0.10 * drift_score
}
