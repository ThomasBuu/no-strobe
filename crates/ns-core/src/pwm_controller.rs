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
/// Banding below this amplitude (luma 0..1, ~1/255) is indistinguishable from sensor noise.
const NOISE_FLOOR: f64 = 0.004;
/// A frame darker than this carries no measurable flicker (everything is noise).
const MIN_LUMA: f64 = 0.02;
/// A blind lock is believed only if the metric at the null is below this fraction of the
/// metric 25 % away on both sides (a true null is |sin(pi e f)|-shaped: ~0 against ~0.7).
const VERIFY_RATIO: f64 = 0.35;
/// Temporal banding (relative to the mean luma) under this is noise, not flicker.
const TEMPORAL_FLOOR: f64 = 0.015;
const SCAN_RATIO: f64 = 1.18;
const SCAN_LINEAR_US: f64 = 1000.0;
const MIN_HZ: f64 = 100.0;
const MAX_HZ: f64 = 5000.0;
/// Frames to spend on one frame rate before dropping to the next: the profile has to be read
/// at that rate long enough to measure its banding period.
const SEARCH_FPS_FRAMES: usize = 24;
/// After the frame-rate descent has found no flicker period in the rows, exposures are swept instead.
/// In a real scene the period does not show in the rows (a fast readout shows about one cycle per
/// frame, drowned in the scene structure), but the exposure sweep with the temporal metric does
/// find the nulls (measured on a real 100 Hz lamp).

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Search,
    /// Hill climb on the banding metric toward an exposure equal to `k / f`.
    /// Runs both when `f` is known (start at `k / f`) and when it is not
    /// (start at the geometric centre of the exposure range).
    Hill,
    /// Short exposure again so the banding profile is readable, then re-estimate f.
    Recalib,
    /// Blind lock candidate: probe 25 % above and below to confirm the null is deep.
    Verify,
    /// Blind scan of the whole exposure range on a log grid, then pick the deepest null.
    Scan,
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
    /// Banding amplitude of the most recent frame, for the offline traces.
    residual_last: f64,
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
    /// Local re-check of an existing lock (blind mode): the state stays Locked while probing
    /// +-2 % around the locked exposure.
    rechecking: bool,
    /// Wanted exposure as a fraction of the frame time (0.5 = 180 degree shutter); 0 = the
    /// shortest k/f. Every multiple of the period cancels the flicker, and a longer one lets
    /// the ISO come down by the same factor.
    shutter_fraction: f64,
    /// Shortest exposure worth searching with: below it the ISO the brightness needs is too high.
    min_exp_floor: f64,
    /// Longest exposure that does not overexpose (brightness budget / lowest ISO); 0 = unknown.
    max_exp_ceiling: f64,
    /// Banding metric summed over the frames of the current trial (averaged at its end).
    trial_acc: f64,
    /// Profiles of the current trial (same exposure): the flicker metric is temporal.
    trial_profiles: Vec<Vec<f64>>,
    temporal_max: f64,
    trial_n: usize,
    verify_stage: u8,
    /// The verification under way is of the first, coarse minimum: f is not set yet.
    verify_coarse: bool,
    verify_frames: usize,
    verify_centre: f64,
    verify_hi_e: f64,
    verify_lo_e: f64,
    verify_hi_m: f64,
    /// Index into `supported_fps` while searching, and frames spent at the current one.
    fps_idx: usize,
    search_frames: usize,
    period_rows: f64,
    period_conf: f64,
    f_from_rows: bool,
    scan_e: f64,
    scan_n: usize,
    scan_res: Vec<(f64, f64)>,
    scan_frames: Vec<Vec<f64>>,
    scan_cands: Vec<f64>,
    scan_window: Option<(f64, f64)>,
    /// Flicker frequency estimated from the spacing of the scan nulls (0 while searching).
    f_est_hz: f64,
}

impl PwmController {
    pub fn new(limits: CameraLimits, line_rate_hz: f64) -> Self {
        let fps = *limits.supported_fps.last().unwrap_or(&30.0);
        let fps_idx = limits.supported_fps.len().saturating_sub(1);
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
            residual_last: 0.0,
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
            rechecking: false,
            shutter_fraction: 0.0,
            min_exp_floor: 0.0,
            max_exp_ceiling: 0.0,
            trial_acc: 0.0,
            trial_profiles: Vec::new(),
            temporal_max: 0.0,
            trial_n: 0,
            verify_stage: 0,
            verify_coarse: false,
            verify_frames: 0,
            verify_centre: 0.0,
            verify_hi_e: 0.0,
            verify_lo_e: 0.0,
            verify_hi_m: 0.0,
            fps_idx,
            search_frames: 0,
            period_rows: 0.0,
            period_conf: 0.0,
            f_from_rows: false,
            scan_e: 0.0,
            scan_n: 0,
            scan_res: Vec::new(),
            scan_frames: Vec::new(),
            scan_cands: Vec::new(),
            scan_window: None,
            f_est_hz: 0.0,
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
        // The row measurement is the period itself. The spacing of the scan nulls is a guess, and
        // it breaks when the deepest scan candidates are short-exposure noise minima: measured at
        // 100 Hz, locked at 10000 us (k=1), the median spacing of the four candidates gave 5714 Hz.
        // The spacing is used only when the profile could not show the period (a period longer
        // than the frame, so the row autocorrelation has nothing to find).
        if self.f_from_rows {
            return self.f_hz;
        }
        // Only a lock gives a frequency: during the scan the commanded exposure is a probe, and
        // 1 / it is noise (the tests check that no frequency is invented from a flat scene).
        if self.phase != Phase::Locked && self.phase != Phase::Recalib {
            return self.f_hz;
        }
        if self.f_hz > 0.0 && self.last_exposure > 0.0 {
            // The controller's own frequency estimate is what the lock was built on: the locked
            // exposure is k / f_hz, so if k is a whole number the estimate is the period. Measured
            // with a 500 Hz light the controller locks at 28001 us (k=14), so f_hz = 500 is
            // confirmed by the lock, and 1 / 28001 us (35.7 Hz) would be 14 times too low - the
            // app's high-speed choice needs fps <= 0.9 f, so 35.7 Hz kills the 240 fps path.
            let k = self.last_exposure * self.f_hz / 1e6;
            if (k - k.round()).abs() < 0.1 {
                return self.f_hz;
            }
        }
        if self.f_est_hz > 0.0 && self.last_exposure > 0.0 {
            // The locked exposure is a whole number of flicker periods, so it must be a multiple
            // of the spacing the scan found. If it is not, the scan's minima were noise: 100 Hz
            // locked at 10000 us, the scan minima gave a spacing of 175 us (5714 Hz), and
            // 10000 x 5714 / 1e6 = 57.14 is not a whole number of periods.
            let k = self.last_exposure * self.f_est_hz / 1e6;
            if (k - k.round()).abs() < 0.1 {
                return self.f_est_hz;
            }
            // The scan's spacing is not the period. The safest number is 1 / the locked exposure:
            // it is the period of the null the controller is sitting on, so the open-loop seed the
            // app applies stays a null, and the high-speed choice is conservative (it picks no
            // frame rate at all when this underestimates f).
            return 1e6 / self.last_exposure;
        }
        if self.last_exposure > 0.0 {
            return 1e6 / self.last_exposure;
        }
        self.f_hz
    }

    /// The hill climb's internal state, for the offline traces: (base, step, probe results).
    pub fn debug_hill(&self) -> (f64, f64, [f64; 3]) {
        (self.hill_base, self.hill_step, self.probe_res)
    }

    /// (best residual recorded at the lock, residual of the last frame).
    /// Last banding period measured in rows, and its autocorrelation.
    pub fn debug_period(&self) -> (f64, f64) {
        (self.period_rows, self.period_conf)
    }

    /// The scan's candidate minima, in the order the scan found them.
    pub fn debug_scan_cands(&self) -> Vec<f64> {
        self.scan_cands.clone()
    }

    /// The exposure the controller last commanded, and the frequency the scan spacing gives.
    pub fn debug_exposure(&self) -> (f64, f64) {
        (self.last_exposure, self.f_est_hz)
    }

    /// The frequency the controller believes, and whether it came from the rows.
    pub fn debug_f(&self) -> (f64, bool) {
        (self.f_hz, self.f_from_rows)
    }

    pub fn debug_residual(&self) -> (f64, f64) {
        (self.best_residual, self.residual_last)
    }

    /// Ask for exposures near `fraction` of the frame time (on a multiple of the flicker period)
    /// once locked. 0 disables it.
    /// Shortest exposure the search may use (the app passes brightness budget / ISO limit). Shorter
    /// exposures need an ISO that makes the preview and the clip noisy, and the nulls of the
    /// flicker repeat at every multiple of its period anyway, so nothing is lost by searching
    /// from here upward.
    pub fn set_min_exposure(&mut self, us: f64) {
        self.min_exp_floor = us.max(0.0);
    }

    /// Longest exposure the scene allows: beyond it even the lowest ISO overexposes the frame.
    pub fn set_max_exposure(&mut self, us: f64) {
        self.max_exp_ceiling = us.max(0.0);
    }

    fn min_exp(&self) -> f64 {
        // Never more than half of the room the search has, or the scan would have no range.
        let floor = self.min_exp_floor.min(0.5 * self.exposure_cap());
        self.limits.min_exposure_us.max(floor).min(self.exposure_cap())
    }

    pub fn set_shutter_fraction(&mut self, fraction: f64) {
        self.shutter_fraction = fraction.clamp(0.0, 1.0);
    }

    /// After a lock on some `k / f`: move to the multiple of the period closest below the wanted
    /// exposure and re-centre there (state stays Locked). Returns true if it moved.
    fn scale_up(&mut self) -> bool {
        if self.shutter_fraction <= 0.0 || self.f_hz <= 0.0 {
            return false;
        }
        let frame_us = 1e6 / self.fps;
        let target = (self.shutter_fraction * frame_us).min(self.exposure_cap());
        // The nearest multiple of the period to the wanted exposure, not the one below: with a
        // wanted 16 ms and a 10 ms period, 20 ms keeps the ISO lower than 10 ms would. Never past
        // what the frame can hold.
        let mut k = (target * self.f_hz / 1e6).round().max(1.0);
        while k > 1.0 && k * 1e6 / self.f_hz > self.exposure_cap() {
            k -= 1.0;
        }
        let e = k * 1e6 / self.f_hz;
        if e <= self.best_exposure * 1.5 {
            return false;
        }
        self.best_exposure = e;
        self.hill_base = e;
        self.hill_exposure = e;
        self.hill_step = 1.04;
        self.probe = 0;
        self.probe_res = [0.0; 3];
        self.trial_acc = 0.0;
        self.trial_n = 0;
        self.frames_since_check = 0;
        self.rechecking = true;
        self.phase = Phase::Hill;
        true
    }

    /// Start already locked on a flicker found earlier (the camera is reopened to record and
    /// must not search again: probing exposures ruin a clip). The usual local re-checks follow.
    pub fn seed_lock(&mut self, exposure_us: f64, f_hz: f64) {
        self.f_hz = f_hz;
        self.confidence = 0.8;
        self.best_exposure = exposure_us;
        // The banding left at a good lock is unknown here: stay tolerant before calling it degraded.
        self.best_residual = 0.02;
        self.flicker_seen = true;
        self.rechecking = false;
        self.frames_since_check = 0;
        self.phase = Phase::Locked;
    }

    /// Feed one frame's vertical luminance profile, get the exposure to apply.
    pub fn on_profile(&mut self, profile: &[f64]) -> crate::controller::Command {
        if self.profiles.len() >= PROFILE_KEEP {
            self.profiles.remove(0);
        }
        self.profiles.push(profile.to_vec());

        match self.phase {
            Phase::Search => self.search(profile),
            Phase::Hill => self.hill(profile),
            Phase::Verify => self.verify(profile),
            Phase::Scan => self.scan(profile),
            Phase::Recalib => self.recalib(profile),
            Phase::Locked => self.locked(profile),
        }
    }

    /// Sweep exposures with the temporal metric, starting at the geometric centre of the range.
    fn start_blind_sweep(&mut self) -> crate::controller::Command {
        self.f_hz = 0.0;
        self.confidence = 0.0;
        self.flicker_seen = false;
        self.trial_profiles.clear();
        self.scan_window = None;
        self.f_est_hz = 0.0;
        self.scan_e = self.hill_min();
        self.scan_n = 0;
        self.scan_res.clear();
        self.scan_frames.clear();
        self.phase = Phase::Scan;
        let e = self.scan_e;
        self.emit(e, State::Search, 0.0)
    }

    /// Next exposure of the scan grid: log-spaced while that is finer than SCAN_LINEAR_US, then in
    /// fixed steps, so that a flicker of a few hundred Hz still has several points per period when
    /// the search starts at a long exposure.
    fn scan_next(&self, e: f64) -> f64 {
        (e + (e * (SCAN_RATIO - 1.0)).min(SCAN_LINEAR_US)).min(self.hill_cap())
    }

    /// Visit every exposure of a log grid (4 frames each, the first is stale), then climb from
    /// the deepest temporal minimum. A local descent from the middle wanders in the flat, noisy
    /// part of the metric; the scan sees all the nulls of the range at once.
    fn scan(&mut self, profile: &[f64]) -> crate::controller::Command {
        let e = self.scan_e;
        self.scan_n += 1;
        if self.scan_n > 1 {
            self.scan_frames.push(profile.to_vec());
        }
        if self.scan_n < 3 {
            return self.emit(e, State::Search, 0.0);
        }
        let m = pwm::temporal_banding(&self.scan_frames);
        let luma = profile.iter().sum::<f64>() / profile.len().max(1) as f64;
        self.scan_frames.clear();
        self.scan_n = 0;
        if luma >= MIN_LUMA {
            self.scan_res.push((e, m));
        }
        if e < self.hill_cap() * 0.999 {
            self.scan_e = self.scan_next(e);
            return self.emit(self.scan_e, State::Search, 0.0);
        }
        // End of the grid.
        let hi = self.scan_res.iter().map(|r| r.1).fold(0.0f64, f64::max);
        // Absolute modulation (relative one times the exposure) is |sin(pi f e)| / (pi f): bounded,
        // and it vanishes only at the nulls (and at e -> 0, an endpoint, never an interior minimum).
        let amp: Vec<f64> = self.scan_res.iter().map(|r| r.1 * r.0).collect();
        let mut cands: Vec<(f64, f64)> = Vec::new();
        for i in 1..amp.len().saturating_sub(1) {
            let nb = amp[i - 1].max(amp[i + 1]);
            if amp[i] <= amp[i - 1] && amp[i] <= amp[i + 1] && nb > 0.0 && amp[i] / nb < 0.85 {
                cands.push((amp[i] / nb, self.scan_res[i].0));
            }
        }
        if self.scan_res.len() < 5 || hi < TEMPORAL_FLOOR || cands.is_empty() {
            // Nothing stands out: no flicker, or too dark. Scan again.
            return self.start_blind_sweep();
        }
        cands.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        // Best candidate last, so `pop` takes the deepest first; keep at most 4.
        let skip = cands.len().saturating_sub(4);
        self.scan_cands = cands.into_iter().skip(skip).map(|c| c.1).collect();
        // The nulls are spaced by the flicker period: two minima 2 ms apart mean 500 Hz, and the
        // locked exposure is k / f, not 1 / f. The app needs f for the high-speed choice (fps <= f)
        // and for the seed; the hill alone only gives one null.
        let mut es = self.scan_cands.clone();
        es.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if es.len() >= 2 {
            let mut sp: Vec<f64> = es
                .windows(2)
                .map(|w| w[1] - w[0])
                .filter(|d| *d > 0.0)
                .collect();
            sp.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            // The largest gap, not the smallest: the false minima the scan finds at the short end
            // (the metric's noise floor) cluster together, so their mutual spacing is far below the
            // period, while the true nulls are one period apart. Measured: 100 Hz, candidates
            // 175, 350, 525, 10000 us -> smallest spacing 175 gives 5714 Hz, largest 9475 gives
            // 105 Hz. On the room lamp (98.7 Hz, minima 1000, 10000, 20000, 30000) the largest
            // gap is 10000 -> 100 Hz.
            let spacing = sp[sp.len() - 1];
            self.f_est_hz = 1e6 / spacing;
        }
        self.next_scan_candidate()
    }

    /// Climb from the next scan minimum, confined to +-25 % so noise cannot drag the climb away
    /// into the flat region; a candidate that fails verification gives way to the next one.
    fn next_scan_candidate(&mut self) -> crate::controller::Command {
        let Some(e) = self.scan_cands.pop() else {
            self.scan_window = None;
            return self.start_blind_sweep();
        };
        self.scan_window = Some((e / 1.25, e * 1.25));
        self.flicker_seen = true;
        self.confidence = 0.3;
        self.hill_base = e;
        self.hill_exposure = e;
        self.hill_step = 1.04;
        self.probe = 0;
        self.probe_res = [0.0; 3];
        self.frames_since_check = 0;
        self.trial_acc = 0.0;
        self.trial_n = 0;
        self.trial_profiles.clear();
        self.temporal_max = 0.0;
        self.phase = Phase::Hill;
        self.emit(self.hill_exposure, State::Search, self.confidence)
    }

    /// Longest exposure the climb may use. Before the flicker is found it stays near one period of
    /// the slowest flicker in range: a long exposure integrates the signal away (relative
    /// modulation ~ 1 / (pi f e)) and the minima lose their contrast against the noise.
    fn hill_cap(&self) -> f64 {
        // Free once the lock is verified (`rechecking`, `scale_up`) or when the row rate is known.
        if self.rechecking || self.line_rate_hz > 0.0 {
            self.exposure_cap()
        } else {
            self.exposure_cap().min((1.2e6 / MIN_HZ).max(self.min_exp() * 2.5))
        }
    }

    fn search(&mut self, profile: &[f64]) -> crate::controller::Command {
        if self.profiles.len() < PROFILE_KEEP {
            return self.emit(self.min_exp(), State::Search, 0.0);
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
                return self.emit(self.min_exp(), State::Search, 0.0);
            }
            return self.emit(self.min_exp(), State::Search, 0.0);
        }
        // No row readout rate (iOS, high-speed): the frame time is known because the controller
        // sets the frame rate, so one row takes frame_us / rows. The banding period measured in
        // rows gives the flicker period in microseconds, and the exposure that cancels it is a
        // multiple of that. Scanning exposures cannot work: the nulls are one period apart, and
        // a grid that does not know the period misses them.
        let rows = profile.len() as f64;
        let frame_us = 1e6 / self.fps;
        let min_rows = rows * (1e6 / MAX_HZ) / frame_us;
        // The profile must show at least three whole periods. A partial period autocorrelates
        // well at a fraction of the true period (the bright pulse overlaps itself), and the
        // controller then locks on a flicker that is three times too fast.
        let max_rows = rows / 3.0;
        if let Some((p, conf)) = pwm::estimate_period_rows(profile, min_rows, max_rows) {
            self.period_rows = p;
            self.period_conf = conf;
            if conf > 0.3 && pwm::banding_amplitude(profile) > NOISE_FLOOR {
                self.f_hz = 1e6 / (p * frame_us / rows);
                self.f_from_rows = true;
                self.confidence = conf;
                self.flicker_seen = true;
                self.search_frames = 0;
                if let Some(e) = self.begin_hill() {
                    return self.emit(e, State::Search, self.confidence);
                }
            }
        }
        // The banding period is longer than the frame, so the profile cannot show it: a period
        // of 5 ms does not fit in a 4,17 ms frame at 240 fps. Drop to the next frame rate, which
        // reads the frame slower and shows more periods. `begin_hill` still picks the highest
        // frame rate the period fits, so the descent only finds the period, it does not choose it.
        self.search_frames += 1;
        if self.search_frames >= SEARCH_FPS_FRAMES {
            self.search_frames = 0;
            if self.fps_idx == 0 {
                // Slowest frame rate reached and still no period in the rows: sweep exposures.
                return self.start_blind_sweep();
            }
            self.fps_idx = self.fps_idx.saturating_sub(1);
            self.fps = self.limits.supported_fps[self.fps_idx];
        }
        self.emit(self.min_exp(), State::Search, 0.0)
    }


    /// Hill climb on the banding metric toward the exposure `k / f`.
    ///
    /// When `f` is known the climb starts at `k / f` and only has to correct the
    /// estimate. When it is not, the climb starts in the middle of the exposure
    /// range and derives `f` from the first minimum it reaches.
    fn hill(&mut self, profile: &[f64]) -> crate::controller::Command {
        self.frames_since_check += 1;
        let progress = if self.rechecking { State::Locked } else { State::Search };
        // Every frame of the trial counts: the average has a third of the noise of one frame.
        self.trial_acc += pwm::banding_metric(profile, self.hill_exposure);
        self.trial_n += 1;
        self.trial_profiles.push(profile.to_vec());
        self.amp_max = self.amp_max.max(pwm::banding_amplitude(profile));
        if self.frames_since_check < TRIAL_FRAMES {
            return self.emit(self.hill_exposure, progress, self.confidence);
        }
        self.frames_since_check = 0;
        // The metric is TEMPORAL (see `temporal_banding`): in a real scene the row-to-row structure
        // is the scene, not the flicker.
        let temporal = pwm::temporal_banding(&self.trial_profiles);
        self.temporal_max = self.temporal_max.max(temporal);
        self.probe_res[self.probe] = if self.trial_profiles.len() >= 3 {
            // No factor of the exposure: relative modulation m|sin(pi f e)|/(pi f e) already falls
            // toward the first null at 1/f; times e it would also vanish at e -> 0 (a false null).
            temporal
        } else {
            self.trial_acc / self.trial_n.max(1) as f64
        };
        self.trial_profiles.clear();
        self.trial_acc = 0.0;
        self.trial_n = 0;
        self.amp_max = self.amp_max.max(pwm::banding_amplitude(profile));
        self.luma_last = profile.iter().sum::<f64>() / profile.len().max(1) as f64;
        self.probe += 1;
        if self.probe < 3 {
            // Probe the centre, then one step up, then one step down.
            let next = if self.probe == 1 {
                (self.hill_base * self.hill_step).min(self.hill_cap())
            } else {
                (self.hill_base / self.hill_step).max(self.hill_min())
            };
            self.hill_exposure = next;
            return self.emit(next, progress, self.confidence);
        }
        self.probe = 0;
        // Is there any flicker to find? Without evidence the three values are noise and
        // the "minimum" is random: stay in Search at the same exposure, never lock.
        let hi = self.probe_res.iter().cloned().fold(0.0f64, f64::max);
        let lo = self.probe_res.iter().cloned().fold(f64::INFINITY, f64::min);
        // Is there flicker at all? Judged on the banding amplitude, not on how different the
        // three probes are: far from a null they differ little (sin near its peak) and the
        // climb still has to move. A claimed minimum is checked separately below.
        let _ = (hi, lo);
        let evidence = self.temporal_max > TEMPORAL_FLOOR && self.luma_last >= MIN_LUMA;
        self.amp_max = 0.0;
        self.temporal_max = 0.0;
        if evidence {
            self.flicker_seen = true;
            // Reported as confidence so the app knows the climb is on a real signal and
            // should be given time to converge (each exposure step costs several frames).
            self.confidence = self.confidence.max(0.3);
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
            _ => {
                // The null lies between the two neighbours. Near a null the metric is
                // V-shaped (|sin|), so the neighbours alone place it: with the null at x0 from
                // the centre, m(-s) - m(+s) = 2 a x0 and m(-s) + m(+s) = 2 a s. The centre
                // itself is left out because noise rounds the bottom of the V.
                let (m_lo, m_hi) = (self.probe_res[2], self.probe_res[1]);
                let sum = m_lo + m_hi;
                if sum > 0.0 {
                    let x0 = ((m_lo - m_hi) / sum).clamp(-1.0, 1.0) * self.hill_step.ln();
                    self.hill_base * x0.exp()
                } else {
                    self.hill_base
                }
            }
        };
        if best_idx == 0 {
            self.hill_step = 1.0 + (self.hill_step - 1.0) * 0.6;
        }
        let (wlo, whi) = self.scan_window.unwrap_or((0.0, f64::INFINITY));
        self.hill_base = best.clamp(self.hill_min(), self.hill_cap()).clamp(wlo, whi);
        self.hill_exposure = self.hill_base;
        if self.rechecking && (self.hill_base / self.best_exposure - 1.0).abs() > 0.15 {
            // The null moved far away: this is no longer the same flicker.
            self.rechecking = false;
            self.profiles.clear();
            self.f_hz = 0.0;
            self.confidence = 0.0;
            self.phase = Phase::Search;
            return self.emit(self.min_exp(), State::Search, 0.0);
        }
        if self.line_rate_hz == 0.0 && self.hill_step < SWEEP_COARSE && self.f_hz == 0.0 {
            // Local banding minimum, but a coarse one (steps of ~5 %) can be a dip of the noise or
            // of the envelope: measured on a real 100 Hz lamp the climb fixed f = 1082 Hz on a
            // shallow dip near 0.9 ms and then refined around nothing. Check 25 % above and below
            // BEFORE trusting it; `verify` sets f when the dip is deep.
            self.verify_coarse = true;
            return self.begin_verify();
        }
        if self.hill_step < HILL_FINE && self.line_rate_hz == 0.0 && !self.rechecking {
            return self.begin_verify();
        }
        if self.hill_step < HILL_FINE {
            let was_rechecking = self.rechecking;
            self.best_exposure = self.hill_base;
            self.best_residual = pwm::banding_amplitude(profile);
            self.rechecking = false;
            self.phase = Phase::Locked;
            self.frames_since_check = 0;
            if !was_rechecking && self.line_rate_hz > 0.0 && self.scale_up() {
                return self.emit(self.hill_exposure, State::Locked, self.confidence);
            }
            return self.emit(self.best_exposure, State::Locked, self.confidence);
        }
        self.emit(self.hill_exposure, progress, self.confidence)
    }

    fn locked(&mut self, profile: &[f64]) -> crate::controller::Command {
        let residual = pwm::banding_amplitude(profile);
        self.residual_last = residual;
        self.frames_since_check += 1;
        let stale = self.frames_since_check >= CHECK_FRAMES;
        let degraded = residual > self.best_residual * (1.0 + RESIDUAL_TOLERANCE)
            && residual > NOISE_FLOOR;
        // Blind mode: re-centre only when the banding really grew. A periodic re-centre probes
        // at +-2 % (and +-4 % right after a scale-up), and every probe changes the exposure the
        // sensor uses, so a recording shows a brightness step each time. Once the base sits on
        // the null, probing costs the video and buys nothing.
        if degraded && self.line_rate_hz == 0.0 {
            self.frames_since_check = 0;
            // The flicker changed (or stopped): search again from scratch.
            if residual > 0.05 {
                self.profiles.clear();
                self.f_hz = 0.0;
                self.confidence = 0.0;
                self.phase = Phase::Search;
                return self.emit(self.min_exp(), State::Search, 0.0);
            }
            // Same flicker, null moved: re-centre, staying at the locked exposure.
            self.hill_base = self.best_exposure;
            self.hill_exposure = self.best_exposure;
            self.hill_step = 1.02;
            self.probe = 0;
            self.probe_res = [0.0; 3];
            self.rechecking = true;
            self.phase = Phase::Hill;
            return self.emit(self.hill_exposure, State::Locked, self.confidence);
        }
        // The line-rate path re-measures f from short-exposure profiles, so it needs a periodic
        // recalibration. Blind mode has no profiles to re-measure: staying on the null is the job.
        if (stale || degraded) && self.line_rate_hz > 0.0 {
            self.frames_since_check = 0;
            self.profiles.clear();
            self.phase = Phase::Recalib;
            // Banding is only readable with a short exposure.
            return self.emit(self.min_exp(), State::Search, self.confidence);
        }
        self.emit(self.best_exposure, State::Locked, self.confidence)
    }

    /// Collect short-exposure profiles, re-estimate f, then refine again.
    /// A blind climb reached a minimum: before believing it, measure the banding 25 % above
    /// and below. Noise gives a "minimum" that is no lower than its surroundings.
    fn begin_verify(&mut self) -> crate::controller::Command {
        self.verify_centre = self.probe_res.iter().cloned().fold(f64::INFINITY, f64::min);
        // Probe half a period away: k*f + T/2 is a half-integer multiple of the period, so its
        // banding is at maximum. A +-25 % probe can land on another null (0.25*k*f is a multiple
        // of T when k is a multiple of 4), and then the deep test compares two nulls and says
        // nothing.
        // A frequency read from the banding period in rows (spatial, scene structure included) can
        // be wrong, so half its period proves nothing: probe a fixed non-harmonic distance.
        let off = if self.f_hz > 0.0 && !self.f_from_rows {
            5e5 / self.f_hz
        } else if self.f_from_rows {
            self.hill_base * 0.37
        } else {
            self.hill_base * 0.25
        };
        self.verify_hi_e = (self.hill_base + off).min(self.exposure_cap());
        self.verify_lo_e = (self.hill_base - off).max(self.hill_min());
        self.verify_stage = 0;
        self.verify_frames = 0;
        self.trial_acc = 0.0;
        self.trial_n = 0;
        self.trial_profiles.clear();
        self.phase = Phase::Verify;
        self.emit(self.verify_hi_e, State::Search, self.confidence)
    }

    fn verify(&mut self, profile: &[f64]) -> crate::controller::Command {
        self.verify_frames += 1;
        let e = if self.verify_stage == 0 { self.verify_hi_e } else { self.verify_lo_e };
        self.trial_acc += pwm::banding_metric(profile, e);
        self.trial_n += 1;
        self.trial_profiles.push(profile.to_vec());
        if self.verify_frames < TRIAL_FRAMES {
            return self.emit(e, State::Search, self.confidence);
        }
        self.verify_frames = 0;
        let m = if self.trial_profiles.len() >= 3 {
            pwm::temporal_banding(&self.trial_profiles)
        } else {
            self.trial_acc / self.trial_n.max(1) as f64
        };
        self.trial_profiles.clear();
        self.trial_acc = 0.0;
        self.trial_n = 0;
        if self.verify_stage == 0 {
            self.verify_hi_m = m;
            self.verify_stage = 1;
            return self.emit(self.verify_lo_e, State::Search, self.confidence);
        }
        // Far probes that really moved (a clamped probe equals the centre and says nothing).
        let mut far = f64::INFINITY;
        let mut far_e = self.hill_base;
        if (self.verify_hi_e / self.hill_base - 1.0).abs() > 0.1 && self.verify_hi_m < far {
            far = self.verify_hi_m;
            far_e = self.verify_hi_e;
        }
        if (self.verify_lo_e / self.hill_base - 1.0).abs() > 0.1 && m < far {
            far = m;
            far_e = self.verify_lo_e;
        }
        let deep = far.is_infinite() || self.verify_centre < VERIFY_RATIO * far;
        if deep && self.verify_coarse {
            // The coarse minimum is a real null: this exposure is k / f for some k. The fine
            // climb continues from here, so k is not assumed to be 1.
            self.verify_coarse = false;
            self.scan_window = None;
            self.f_hz = 1e6 / self.hill_base;
            self.f_from_rows = false;
            self.confidence = 0.5;
            self.hill_step = 1.02;
            self.probe = 0;
            self.probe_res = [0.0; 3];
            self.frames_since_check = 0;
            self.trial_acc = 0.0;
            self.trial_n = 0;
            self.trial_profiles.clear();
            self.phase = Phase::Hill;
            self.hill_exposure = self.hill_base;
            return self.emit(self.hill_exposure, State::Search, self.confidence);
        }
        if deep {
            self.best_exposure = self.hill_base;
            self.best_residual = (self.verify_centre / self.hill_base).max(0.002);
            self.rechecking = false;
            self.phase = Phase::Locked;
            self.frames_since_check = 0;
            if self.scale_up() {
                return self.emit(self.hill_exposure, State::Locked, self.confidence);
            }
            return self.emit(self.best_exposure, State::Locked, self.confidence);
        }
        if self.scan_window.is_some() {
            self.verify_coarse = false;
            return self.next_scan_candidate();
        }
        // Not a null: forget this frequency and keep climbing from the lower far probe.
        self.verify_coarse = false;
        self.f_from_rows = false;
        self.f_hz = 0.0;
        self.confidence = 0.3;
        self.hill_step = 1.25;
        self.hill_base = far_e.clamp(self.hill_min(), self.exposure_cap());
        self.hill_exposure = self.hill_base;
        self.probe = 0;
        self.probe_res = [0.0; 3];
        self.frames_since_check = 0;
        self.phase = Phase::Hill;
        self.emit(self.hill_exposure, State::Search, self.confidence)
    }

    fn recalib(&mut self, _profile: &[f64]) -> crate::controller::Command {
        if self.profiles.len() < PROFILE_KEEP {
            return self.emit(self.min_exp(), State::Search, self.confidence);
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
                return self.emit(self.min_exp(), State::Search, 0.0);
            }
            // No readable flicker: the light stopped. Keep the last exposure.
            self.phase = Phase::Locked;
            self.frames_since_check = 0;
            return self.emit(self.best_exposure, State::Locked, self.confidence);
        }
        self.phase = Phase::Search;
        self.emit(self.min_exp(), State::Search, 0.0)
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

    /// Shortest exposure the climb may use. Below `1 / MAX_HZ` an exposure cannot be `k / f`
    /// for any flicker in range, and the banding metric (amplitude x exposure) goes to zero
    /// there without proving anything: a climb that drifts to the shortest exposure "finds"
    /// a minimum that is not one.
    fn hill_min(&self) -> f64 {
        self.min_exp().max(1e6 / MAX_HZ).min(self.exposure_cap())
    }

    /// Longest exposure the frame can hold: a sensor accepts exposures far longer than one
    /// frame, but then stretches the frame duration and the frame rate collapses.
    fn exposure_cap(&self) -> f64 {
        let frame_us = 1e6 / self.fps;
        let mut cap = self
            .limits
            .max_exposure_us
            .min((frame_us - self.limits.sustain_us).max(self.limits.min_exposure_us));
        if self.max_exp_ceiling > 0.0 {
            cap = cap.min(self.max_exp_ceiling.max(self.limits.min_exposure_us));
        }
        cap
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
