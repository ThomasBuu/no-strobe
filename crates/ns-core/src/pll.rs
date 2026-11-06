//! Phase-Locked Loop (PLL) and predictive tracking for rolling shutter flicker.
//!
//! Compensates for Android Camera2 hardware pipeline latency (REQUEST_PIPELINE_MAX_DEPTH,
//! typically 2 to 3 frames), ensuring that commanded exposure and phase synchronization
//! match the physical sensor exposure timing without lag or hunting.

#[derive(Debug, Clone, Copy)]
pub struct PllConfig {
    /// Phase loop gain (proportional). Default 0.25
    pub kp: f64,
    /// Frequency loop gain (integral). Default 0.02
    pub ki: f64,
    /// Hardware camera pipeline depth in frames (default 2.0 frames)
    pub pipeline_depth: f64,
}

impl Default for PllConfig {
    fn default() -> Self {
        Self {
            kp: 0.35,
            ki: 0.15,
            pipeline_depth: 2.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FlickerPll {
    config: PllConfig,
    /// Frequency in Hz
    f_hz: f64,
    /// Phase in radians [0, 2*PI)
    phase_rad: f64,
    /// Last update timestamp in microseconds
    last_t_us: f64,
    /// Phase jitter / variance
    jitter_rad: f64,
    /// Confidence in lock (0.0 .. 1.0)
    confidence: f64,
    initialized: bool,
}

impl Default for FlickerPll {
    fn default() -> Self {
        Self::new(PllConfig::default())
    }
}

impl FlickerPll {
    pub fn new(config: PllConfig) -> Self {
        Self {
            config,
            f_hz: 0.0,
            phase_rad: 0.0,
            last_t_us: 0.0,
            jitter_rad: 0.0,
            confidence: 0.0,
            initialized: false,
        }
    }

    pub fn init(&mut self, f_hz: f64, phase_rad: f64, t_us: f64) {
        self.f_hz = f_hz.max(1.0);
        self.phase_rad = phase_rad.rem_euclid(2.0 * std::f64::consts::PI);
        self.last_t_us = t_us;
        self.jitter_rad = 0.0;
        self.confidence = 0.5;
        self.initialized = true;
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn frequency_hz(&self) -> f64 {
        self.f_hz
    }

    pub fn phase_rad(&self) -> f64 {
        self.phase_rad
    }

    pub fn confidence(&self) -> f64 {
        self.confidence
    }

    pub fn set_pipeline_depth(&mut self, depth: f64) {
        self.config.pipeline_depth = depth.clamp(0.0, 5.0);
    }

    pub fn pipeline_depth(&self) -> f64 {
        self.config.pipeline_depth
    }

    /// Predict phase forward to timestamp `target_t_us` in microseconds.
    pub fn predict_phase_at(&self, target_t_us: f64) -> f64 {
        if !self.initialized {
            return self.phase_rad;
        }
        let dt_s = (target_t_us - self.last_t_us) * 1e-6;
        let d_phase = 2.0 * std::f64::consts::PI * self.f_hz * dt_s;
        (self.phase_rad + d_phase).rem_euclid(2.0 * std::f64::consts::PI)
    }

    /// Predict phase forward by `pipeline_depth` frames into the future.
    pub fn predict_lead_phase(&self, frame_period_us: f64) -> f64 {
        let target_t = self.last_t_us + self.config.pipeline_depth * frame_period_us;
        self.predict_phase_at(target_t)
    }

    /// Update the PLL with a phase measurement from the current frame.
    /// `measured_phase_rad`: phase in radians [0, 2*PI).
    /// `t_us`: timestamp of the frame in microseconds.
    /// `quality`: measurement quality weight in [0.0, 1.0].
    pub fn update(&mut self, measured_phase_rad: f64, t_us: f64, quality: f64) {
        if !self.initialized {
            self.init(self.f_hz.max(100.0), measured_phase_rad, t_us);
            return;
        }

        let dt_s = (t_us - self.last_t_us) * 1e-6;
        if dt_s <= 0.0 || dt_s > 1.0 {
            // Gap or clock discontinuity: re-align phase, keep frequency
            self.phase_rad = measured_phase_rad.rem_euclid(2.0 * std::f64::consts::PI);
            self.last_t_us = t_us;
            return;
        }

        // 1. Time update (Prediction)
        let pred_phase = (self.phase_rad + 2.0 * std::f64::consts::PI * self.f_hz * dt_s)
            .rem_euclid(2.0 * std::f64::consts::PI);

        // 2. Phase error with wrap-around to [-PI, PI]
        let mut err = (measured_phase_rad - pred_phase).rem_euclid(2.0 * std::f64::consts::PI);
        if err > std::f64::consts::PI {
            err -= 2.0 * std::f64::consts::PI;
        }

        // 3. Loop filter update
        let weight = quality.clamp(0.0, 1.0);
        let kp = self.config.kp * weight;
        let ki = self.config.ki * weight;

        // Correct phase
        self.phase_rad = (pred_phase + kp * err).rem_euclid(2.0 * std::f64::consts::PI);

        // Correct frequency: delta_f = ki * err / (2*PI * dt)
        let df = (ki * err) / (2.0 * std::f64::consts::PI * dt_s.max(1e-4));
        let max_df = 0.05 * self.f_hz;
        self.f_hz = (self.f_hz + df.clamp(-max_df, max_df)).max(10.0);

        // 4. Jitter tracking & confidence
        self.jitter_rad = 0.85 * self.jitter_rad + 0.15 * err.abs();
        let conf = (1.0 - (self.jitter_rad / std::f64::consts::FRAC_PI_2)).clamp(0.0, 1.0);
        self.confidence = 0.9 * self.confidence + 0.1 * conf;
        self.last_t_us = t_us;
    }
}

/// Measures the phase of a vertical profile at a known row period in radians [0, 2*PI).
pub fn measure_profile_phase(profile: &[f64], period_rows: f64) -> Option<f64> {
    let n = profile.len();
    if n < 8 || period_rows <= 1.0 {
        return None;
    }
    let mean = profile.iter().sum::<f64>() / n as f64;
    let omega = 2.0 * std::f64::consts::PI / period_rows;
    let mut sum_re = 0.0;
    let mut sum_im = 0.0;
    for (y, &val) in profile.iter().enumerate() {
        let v = val - mean;
        let theta = omega * y as f64;
        sum_re += v * theta.cos();
        sum_im += v * theta.sin();
    }
    let mag = (sum_re * sum_re + sum_im * sum_im).sqrt();
    if mag <= 1e-6 {
        return None;
    }
    Some(sum_re.atan2(sum_im).rem_euclid(2.0 * std::f64::consts::PI))
}
