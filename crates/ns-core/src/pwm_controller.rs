//! Closed loop for high-frequency PWM flicker (LED projectors, stage screens).
//!
//! The frame-mean luminance aliases above ~fps/2, so the flicker is read from
//! the vertical luminance profile: a rolling shutter scans rows at the readout
//! rate, so banding in the profile is the flicker sampled in time.
//!
//! Loop: estimate `f` from the profile spectrum, try exposures around `k / f`,
//! keep the one with the least residual banding, re-check periodically because
//! PWM duty and frequency change during a show.

use crate::camera::CameraLimits;
use crate::controller::State;
use crate::pwm;
use crate::solver::Mode;

/// Frames spent measuring one probe exposure.
const TRIAL_FRAMES: usize = 3;
/// Frames between re-checks while locked.
const CHECK_FRAMES: usize = 30;
/// Profiles kept for the spectrum estimate.
const PROFILE_KEEP: usize = 8;
/// Banding above this fraction of the locked value means the lock is off.
const RESIDUAL_TOLERANCE: f64 = 0.4;
/// Sweep step where an unknown row readout rate is enough to derive f.
const SWEEP_COARSE: f64 = 1.05;
/// Hill climb step that is precise enough to lock (0.2 % on the exposure).
const HILL_FINE: f64 = 1.002;
/// A banding minimum only counts if the metric really dips: the three probes must differ by
/// this fraction of the largest one. Pure sensor noise gives near-equal, random values.
const EVIDENCE_SPREAD: f64 = 0.25;
/// Banding below this amplitude (luma 0..1, ~1/255) is indistinguishable from sensor noise.
const NOISE_FLOOR: f64 = 0.004;
/// A frame darker than this carries no measurable flicker (everything is noise).
const MIN_LUMA: f64 = 0.02;
const MIN_HZ: f64 = 100.0;
const MAX_HZ: f64 = 5000.0;

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Search,
    /// Hill climb on the banding metric toward an exposure equal to `k / f`.
    /// Runs both when `f` is known (start at `k / f`) and when it is not
    /// (start at the geometric centre of the exposure range).
    Hill,
    /// Short exposure again so the banding profile is readable, then re-estimate f.
    Recalib,
    Locked,
}

pub struct PwmController {
    limits: CameraLimits,
    line_rate_hz: f64,
    fps: f64,
    profiles: Vec<Vec<f64>>,
    phase: Phase,
    f_hz: f64,
    confidence: f64,
    best_exposure: f64,
    best_residual: f64,
    frames_since_check: usize,
    last_exposure: f64,
    /// Hill climb on the banding metric: centre exposure, multiplicative step,
    /// probe slot 0 = centre, 1 = centre * step, 2 = centre / step.
    hill_exposure: f64,
    hill_base: f64,
    hill_step: f64,
    probe: usize,
    probe_res: [f64; 3],
    amp_max: f64,
    luma_last: f64,
    /// Flicker was shown to exist (evidence gate passed, or `f` measured on the profile).
    /// Near a true null the banding is tiny by design, so the gate only applies before.
    flicker_seen: bool,
}

impl PwmController {
    pub fn new(limits: CameraLimits, line_rate_hz: f64) -> Self {
        let fps = *limits.supported_fps.last().unwrap_or(&30.0);
        Self {
            limits,
            line_rate_hz,
            fps,
            profiles: Vec::new(),
            phase: Phase::Search,
            f_hz: 0.0,
            confidence: 0.0,
            best_exposure: 0.0,
            best_residual: 0.0,
            frames_since_check: 0,
            last_exposure: 0.0,
            hill_exposure: 0.0,
            hill_base: 0.0,
            hill_step: 0.0,
            probe: 0,
            probe_res: [0.0; 3],
            amp_max: 0.0,
            luma_last: 0.0,
            flicker_seen: false,
        }
    }

    pub fn state(&self) -> State {
        if self.phase == Phase::Locked {
            State::Locked
        } else {
            State::Search
        }
    }

    pub fn frequency_hz(&self) -> f64 {
        self.f_hz
    }

    /// Feed one frame's vertical luminance profile, get the exposure to apply.
    pub fn on_profile(&mut self, profile: &[f64]) -> crate::controller::Command {
        if self.profiles.len() >= PROFILE_KEEP {
            self.profiles.remove(0);
        }
        self.profiles.push(profile.to_vec());

        match self.phase {
            Phase::Search => self.search(),
            Phase::Hill => self.hill(profile),
            Phase::Recalib => self.recalib(profile),
            Phase::Locked => self.locked(profile),
        }
    }

    fn search(&mut self) -> crate::controller::Command {
        if self.profiles.len() < PROFILE_KEEP {
            return self.emit(self.limits.min_exposure_us, State::Search, 0.0);
        }
        let mean = self.mean_profile();
        if self.line_rate_hz > 0.0 {
            if let Some((f, conf)) = pwm::estimate_f(&mean, self.line_rate_hz, MIN_HZ, MAX_HZ) {
                self.f_hz = f;
                self.confidence = conf;
                if let Some(e) = self.begin_hill() {
                    return self.emit(e, State::Search, self.confidence);
                }
                // No exposure in the device range fits: keep searching.
                return self.emit(self.limits.min_exposure_us, State::Search, 0.0);
            }
            return self.emit(self.limits.min_exposure_us, State::Search, 0.0);
        }
        // No row readout rate (iOS): hill-climb the banding metric. Its minima sit
        // at exposures equal to k / f, so f is not needed to find them. Start at
        // the geometric centre of the range so the climb can move both ways.
        self.hill_base = (self.limits.min_exposure_us * self.exposure_cap()).sqrt();
        self.hill_exposure = self.hill_base;
        self.hill_step = 1.25;
        self.probe = 0;
        self.probe_res = [0.0; 3];
        self.flicker_seen = false;
        self.phase = Phase::Hill;
        self.emit(self.hill_exposure, State::Search, 0.0)
    }

    /// Hill climb on the banding metric toward the exposure `k / f`.
    ///
    /// When `f` is known the climb starts at `k / f` and only has to correct the
    /// estimate. When it is not, the climb starts in the middle of the exposure
    /// range and derives `f` from the first minimum it reaches.
    fn hill(&mut self, profile: &[f64]) -> crate::controller::Command {
        self.frames_since_check += 1;
        if self.frames_since_check < TRIAL_FRAMES {
            return self.emit(self.hill_exposure, State::Search, self.confidence);
        }
        self.frames_since_check = 0;
        self.probe_res[self.probe] = pwm::banding_metric(profile, self.hill_exposure);
        self.amp_max = self.amp_max.max(pwm::banding_amplitude(profile));
        self.luma_last = profile.iter().sum::<f64>() / profile.len().max(1) as f64;
        self.probe += 1;
        if self.probe < 3 {
            // Probe the centre, then one step up, then one step down.
            let next = if self.probe == 1 {
                (self.hill_base * self.hill_step).min(self.exposure_cap())
            } else {
                (self.hill_base / self.hill_step).max(self.limits.min_exposure_us)
            };
            self.hill_exposure = next;
            return self.emit(next, State::Search, self.confidence);
        }
        self.probe = 0;
        // Is there any flicker to find? Without evidence the three values are noise and
        // the "minimum" is random: stay in Search at the same exposure, never lock.
        let hi = self.probe_res.iter().cloned().fold(0.0f64, f64::max);
        let lo = self.probe_res.iter().cloned().fold(f64::INFINITY, f64::min);
        let evidence = hi > 0.0
            && (hi - lo) / hi > EVIDENCE_SPREAD
            && self.amp_max > NOISE_FLOOR
            && self.luma_last >= MIN_LUMA;
        self.amp_max = 0.0;
        if evidence {
            self.flicker_seen = true;
        }
        if !evidence && !self.flicker_seen {
            self.hill_step = 1.25;
            self.hill_exposure = self.hill_base;
            return self.emit(self.hill_exposure, State::Search, 0.0);
        }
        // Three samples: centre, *step, /step. Move to the best; shrink the step
        // when the centre is already the local minimum.
        let mut best_idx = 0usize;
        for (i, r) in self.probe_res.iter().enumerate() {
            if *r < self.probe_res[best_idx] {
                best_idx = i;
            }
        }
        let best = match best_idx {
            1 => self.hill_base * self.hill_step,
            2 => self.hill_base / self.hill_step,
            _ => self.hill_base,
        };
        if best_idx == 0 {
            self.hill_step = 1.0 + (self.hill_step - 1.0) * 0.6;
        }
        self.hill_base = best.clamp(self.limits.min_exposure_us, self.exposure_cap());
        self.hill_exposure = self.hill_base;
        if self.line_rate_hz == 0.0 && self.hill_step < SWEEP_COARSE && self.f_hz == 0.0 {
            // Local banding minimum: this exposure is k / f for some k. The climb
            // continues at a finer step, so k is not assumed to be 1.
            self.f_hz = 1e6 / self.hill_base;
            self.confidence = 0.5;
            self.hill_step = 1.02;
        }
        if self.hill_step < HILL_FINE {
            self.best_exposure = self.hill_base;
            self.best_residual = pwm::banding_amplitude(profile);
            self.phase = Phase::Locked;
            self.frames_since_check = 0;
            return self.emit(self.best_exposure, State::Locked, self.confidence);
        }
        self.emit(self.hill_exposure, State::Search, self.confidence)
    }

    fn locked(&mut self, profile: &[f64]) -> crate::controller::Command {
        let residual = pwm::banding_amplitude(profile);
        self.frames_since_check += 1;
        let stale = self.frames_since_check >= CHECK_FRAMES;
        let degraded = residual > self.best_residual * (1.0 + RESIDUAL_TOLERANCE)
            && residual > 0.01;
        if stale || degraded {
            self.frames_since_check = 0;
            self.profiles.clear();
            self.phase = Phase::Recalib;
            // Banding is only readable with a short exposure.
            return self.emit(self.limits.min_exposure_us, State::Search, self.confidence);
        }
        self.emit(self.best_exposure, State::Locked, self.confidence)
    }

    /// Collect short-exposure profiles, re-estimate f, then refine again.
    fn recalib(&mut self, _profile: &[f64]) -> crate::controller::Command {
        if self.profiles.len() < PROFILE_KEEP {
            return self.emit(self.limits.min_exposure_us, State::Search, self.confidence);
        }
        let mean = self.mean_profile();
        if self.line_rate_hz > 0.0 {
            if let Some((f, conf)) = pwm::estimate_f(&mean, self.line_rate_hz, MIN_HZ, MAX_HZ) {
                self.f_hz = f;
                self.confidence = conf;
                if let Some(e) = self.begin_hill() {
                    return self.emit(e, State::Search, self.confidence);
                }
                self.phase = Phase::Search;
                return self.emit(self.limits.min_exposure_us, State::Search, 0.0);
            }
            // No readable flicker: the light stopped. Keep the last exposure.
            self.phase = Phase::Locked;
            self.frames_since_check = 0;
            return self.emit(self.best_exposure, State::Locked, self.confidence);
        }
        self.phase = Phase::Search;
        self.emit(self.limits.min_exposure_us, State::Search, 0.0)
    }

    /// Start a hill climb at the largest `k / f` that fits the frame period.
    /// The largest `k` keeps the most light; the climb corrects the estimate.
    fn begin_hill(&mut self) -> Option<f64> {
        let mut chosen = None;
        for fps in self.limits.supported_fps.iter().rev() {
            let frame_us = 1e6 / fps;
            for k in 1..64u32 {
                let e = pwm::exposure_for_k(self.f_hz, k);
                if e > self.limits.max_exposure_us {
                    break;
                }
                if e >= self.limits.min_exposure_us && e + self.limits.sustain_us <= frame_us {
                    chosen = Some((*fps, e));
                }
            }
            if chosen.is_some() {
                break;
            }
        }
        let (fps, e) = chosen?;
        self.fps = fps;
        self.hill_base = e;
        self.hill_exposure = e;
        self.hill_step = 1.02;
        self.probe = 0;
        self.probe_res = [0.0; 3];
        self.frames_since_check = 0;
        self.flicker_seen = true;
        self.phase = Phase::Hill;
        Some(e)
    }

    fn mean_profile(&self) -> Vec<f64> {
        let n = self.profiles.iter().map(|p| p.len()).min().unwrap_or(0);
        let mut out = vec![0.0; n];
        for p in &self.profiles {
            for (i, v) in p.iter().enumerate().take(n) {
                out[i] += v;
            }
        }
        let k = self.profiles.len().max(1) as f64;
        out.iter().map(|v| v / k).collect()
    }

    /// Longest exposure the frame can hold: a sensor accepts exposures far longer than one
    /// frame, but then stretches the frame duration and the frame rate collapses.
    fn exposure_cap(&self) -> f64 {
        let frame_us = 1e6 / self.fps;
        self.limits
            .max_exposure_us
            .min((frame_us - self.limits.sustain_us).max(self.limits.min_exposure_us))
    }

    fn emit(&mut self, exposure_us: f64, state: State, confidence: f64) -> crate::controller::Command {
        self.last_exposure = exposure_us;
        let iso = (self.limits.min_iso * (self.limits.max_exposure_us / exposure_us.max(1.0)))
            .clamp(self.limits.min_iso, self.limits.max_iso);
        crate::controller::Command {
            exposure_us,
            fps: self.fps,
            iso,
            mode: Mode::Pwm,
            state,
            period_us: if self.f_hz > 0.0 { 1e6 / self.f_hz } else { 0.0 },
            confidence,
            drift_us_per_frame: 0.0,
        }
    }
}
