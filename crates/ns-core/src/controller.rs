use crate::camera::CameraLimits;
use crate::detector::{coarse_period, fit_from_pulses, PulseTracker};
use crate::signal::{Frame, SignalBuffer};
use crate::solver::{solve, Mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Short exposure, collecting a flicker signal.
    Search,
    /// Strobe period known, exposure locked to it.
    Locked,
}

#[derive(Debug, Clone, Copy)]
pub struct Command {
    pub exposure_us: f64,
    pub fps: f64,
    pub iso: f64,
    pub mode: Mode,
    pub state: State,
    pub period_us: f64,
    pub confidence: f64,
    pub drift_us_per_frame: f64,
}

impl Default for Command {
    fn default() -> Self {
        Self {
            exposure_us: 0.0,
            fps: 0.0,
            iso: 0.0,
            mode: Mode::Flash,
            state: State::Search,
            period_us: 0.0,
            confidence: 0.0,
            drift_us_per_frame: 0.0,
        }
    }
}

/// Short exposure used while searching. A long exposure integrates whole strobe cycles and
/// flattens the signal, so detection needs a narrow sampling window.
const SEARCH_EXPOSURE_US: f64 = 8_000.0;

pub struct Controller {
    pub limits: CameraLimits,
    pub buf: SignalBuffer,
    pub tracker: Option<PulseTracker>,
    pub state: State,
    pub warmup_frames: usize,
    pub last_command: Command,
}

impl Controller {
    pub fn new(limits: CameraLimits) -> Self {
        let fps = limits
            .supported_fps
            .iter()
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .copied()
            .unwrap_or(30.0);
        let exposure = search_exposure(&limits);
        Self {
            limits,
            buf: SignalBuffer::new(120),
            tracker: None,
            state: State::Search,
            warmup_frames: 45,
            last_command: Command {
                exposure_us: exposure,
                fps,
                ..Command::default()
            },
        }
    }

    pub fn on_frame(&mut self, t_us: f64, luminance: f64) -> Command {
        self.buf.push(Frame { t_us, luminance });
        if self.state == State::Search {
            self.search()
        } else {
            self.track()
        }
    }

    fn stats(&self) -> (f64, f64) {
        let lum: Vec<f64> = self.buf.frames().map(|f| f.luminance).collect();
        if lum.is_empty() {
            return (0.0, 0.0);
        }
        let mean = lum.iter().sum::<f64>() / lum.len() as f64;
        let var = lum.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / lum.len() as f64;
        (mean, var.sqrt())
    }

    fn search(&mut self) -> Command {
        if self.buf.len() < self.warmup_frames {
            self.last_command = Command {
                exposure_us: search_exposure(&self.limits),
                fps: self.last_command.fps,
                ..Command::default()
            };
            return self.last_command;
        }

        let frame_us = self.buf.frame_period_us();
        let det = self.buf.detrended(15);
        // The search band is bounded by the sampling rate: frame-mean luma sampled at fps
        // cannot resolve a strobe above fps/2.
        let max_hz = (1e6 / frame_us / 2.0 * 0.8).min(60.0);
        let coarse = coarse_period(&det, frame_us, 4.0, max_hz);

        let (mean, std) = self.stats();
        let threshold = mean + 0.6 * std;
        let pulses = self.buf.rising_edge_times(threshold);

        let guess = coarse.map(|(p, _)| p).unwrap_or(frame_us * 2.0);
        if let Some(est) = fit_from_pulses(&pulses, guess, frame_us) {
            if est.confidence >= 0.45 && pulses.len() >= 4 {
                self.tracker = Some(PulseTracker::new(est));
                self.state = State::Locked;
                return self.track();
            }
        }

        self.last_command = Command {
            exposure_us: search_exposure(&self.limits),
            fps: self.last_command.fps,
            ..Command::default()
        };
        self.last_command
    }

    fn track(&mut self) -> Command {
        let (mean, std) = self.stats();
        let threshold = mean + 0.6 * std;
        let pulses = self.buf.rising_edge_times(threshold);

        if let Some(tracker) = &mut self.tracker {
            tracker.update(&pulses);
        }

        let est = match self.tracker.as_ref() {
            Some(tracker) => tracker.est,
            None => {
                self.state = State::Search;
                return self.search();
            }
        };

        let candidates = solve(&est, &self.limits);
        if candidates.is_empty() {
            self.state = State::Search;
            return self.search();
        }

        let best = candidates[0];
        // Gap and Flash throw away light, so gain has to make up the difference.
        let iso = (self.limits.min_iso * (self.limits.max_exposure_us / best.exposure_us))
            .clamp(self.limits.min_iso, self.limits.max_iso);
        self.last_command = Command {
            exposure_us: best.exposure_us,
            fps: best.fps,
            iso,
            mode: best.mode,
            state: State::Locked,
            period_us: est.period_us,
            confidence: est.confidence,
            drift_us_per_frame: best.drift_us_per_frame,
        };
        self.last_command
    }
}

fn search_exposure(limits: &CameraLimits) -> f64 {
    limits.min_exposure_us.max(limits.max_exposure_us.min(SEARCH_EXPOSURE_US))
}
