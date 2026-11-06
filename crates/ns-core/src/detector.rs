#[derive(Debug, Clone, Copy)]
pub struct Estimate {
    /// Strobe period, microseconds.
    pub period_us: f64,
    /// Absolute time of a flash, used as the phase reference.
    pub phase_us: f64,
    /// Flash width divided by period.
    pub duty: f64,
    /// 0..1, how clean the periodicity is.
    pub confidence: f64,
}

/// Normalized autocorrelation over fractional lags, with parabolic peak refinement.
/// Returns (period_us, correlation score).
pub fn coarse_period(
    detrended: &[f64],
    frame_period_us: f64,
    min_hz: f64,
    max_hz: f64,
) -> Option<(f64, f64)> {
    let n = detrended.len();
    if n < 12 || frame_period_us <= 0.0 {
        return None;
    }

    // Lag is measured in frames. Period in frames = period_us / frame_period_us.
    let min_lag = 1e6 / (max_hz * frame_period_us);
    let max_lag = (1e6 / (min_hz * frame_period_us)).min((n - 1) as f64);
    if min_lag >= max_lag {
        return None;
    }

    let energy: f64 = detrended.iter().map(|v| v * v).sum();
    if energy <= 1e-9 {
        return None;
    }

    let step = 0.05;
    let mut best_lag = min_lag;
    let mut best = f64::MIN;
    let mut lag = min_lag;
    while lag <= max_lag {
        let c = autocorr(detrended, lag) / energy;
        if c > best {
            best = c;
            best_lag = lag;
        }
        lag += step;
    }

    let y0 = autocorr(detrended, best_lag - step) / energy;
    let y2 = autocorr(detrended, best_lag + step) / energy;
    let denom = y0 - 2.0 * best + y2;
    let refined = if denom.abs() > 1e-12 {
        best_lag - 0.5 * (y2 - y0) / denom
    } else {
        best_lag
    };

    Some((refined * frame_period_us, best))
}

fn autocorr(x: &[f64], lag: f64) -> f64 {
    let n = x.len() as f64;
    let mut sum = 0.0;
    let mut i = 0.0;
    while i + lag < n {
        sum += x[i as usize] * interp(x, i + lag);
        i += 1.0;
    }
    sum
}

fn interp(x: &[f64], pos: f64) -> f64 {
    let i = pos.floor() as usize;
    if i + 1 >= x.len() {
        return x[x.len() - 1];
    }
    let f = pos - i as f64;
    x[i] * (1.0 - f) + x[i + 1] * f
}

/// Least-squares fit of flash times to `phase + m * period`, where m is an integer cycle
/// count. The seed comes from the autocorrelation guess, refined by the pulse spacings
/// `median / k`. Two guards keep the seed honest: spacings equal to the frame period are a
/// sampling artifact, not the strobe, and a seed far from the autocorrelation peak means the
/// spacings are aliased, so the peak wins.
/// Times of rising edges: the first frame of each run above `threshold`.
///
/// A square wave whose bright part spans several frames puts every one of them
/// above the threshold, so plain pulse detection yields several pulses per
/// cycle and the integer cycle counts collapse. The rising edge is one per
/// cycle and is what the phase fit needs.
pub fn rising_edges(luma: &[f64], threshold: f64, frame_us: f64) -> Vec<f64> {
    let mut out = Vec::new();
    let mut above = false;
    for (i, v) in luma.iter().enumerate() {
        let now = *v > threshold;
        if now && !above {
            out.push(i as f64 * frame_us);
        }
        above = now;
    }
    out
}

pub fn fit_from_pulses(
    pulses: &[f64],
    guess_period_us: f64,
    frame_period_us: f64,
) -> Option<Estimate> {
    if pulses.len() < 3 || guess_period_us <= 0.0 {
        return None;
    }

    let mut diffs: Vec<f64> = pulses.windows(2).map(|w| w[1] - w[0]).collect();
    diffs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = diffs[diffs.len() / 2];
    if median <= 0.0 {
        return None;
    }

    let floor_us = 1.2 * frame_period_us;
    let mut best: Option<f64> = None;
    for k in 1..=4 {
        let cand = median / k as f64;
        if cand < floor_us {
            continue;
        }
        match best {
            Some(b) if (b - guess_period_us).abs() <= (cand - guess_period_us).abs() => {}
            _ => best = Some(cand),
        }
    }
    // A spacing candidate only refines the peak when it is near it. Far away means the
    // spacing grid is aliased against the frame rate, so the peak stands.
    let seed = match best {
        Some(c) if (c - guess_period_us).abs() <= 0.25 * guess_period_us => c,
        _ => guess_period_us,
    };

    let t0 = pulses[0];
    let mut period = seed;
    let mut phase = t0;
    let mut confidence = 0.0;

    for _ in 0..4 {
        let cycles: Vec<f64> = pulses.iter().map(|p| (p - t0) / period).collect();
        let m: Vec<f64> = cycles.iter().map(|c| c.round()).collect();

        let n = pulses.len() as f64;
        let sm: f64 = m.iter().sum();
        let sp: f64 = pulses.iter().sum();
        let smm: f64 = m.iter().map(|x| x * x).sum();
        let smp: f64 = m.iter().zip(pulses.iter()).map(|(a, b)| a * b).sum();
        let denom = n * smm - sm * sm;
        if denom.abs() < 1e-9 {
            return None;
        }

        period = (n * smp - sm * sp) / denom;
        phase = (sp - period * sm) / n;

        if period <= 0.0 {
            return None;
        }

        let rms = pulses
            .iter()
            .zip(m.iter())
            .map(|(p, mi)| (p - (phase + mi * period)).powi(2))
            .sum::<f64>()
            / n;
        confidence = 1.0 - (rms.sqrt() / period).clamp(0.0, 1.0);
    }

    Some(Estimate {
        period_us: period,
        phase_us: phase,
        duty: 0.1,
        confidence,
    })
}

/// Keeps the estimate tracking a strobe whose tempo drifts (beat-synced strobes do).
pub struct PulseTracker {
    pub est: Estimate,
    pub frames_since_pulse: usize,
}

impl PulseTracker {
    pub fn new(est: Estimate) -> Self {
        Self {
            est,
            frames_since_pulse: 0,
        }
    }

    pub fn update(&mut self, pulses: &[f64]) {
        if pulses.is_empty() {
            self.frames_since_pulse += 1;
            if self.frames_since_pulse > 30 {
                self.est.confidence *= 0.9;
            }
            return;
        }

        for &p in pulses {
            let m = ((p - self.est.phase_us) / self.est.period_us).round();
            let residual = p - (self.est.phase_us + m * self.est.period_us);
            self.est.phase_us += 0.05 * residual;
            self.est.period_us += 0.002 * residual;
            self.est.confidence = self.est.confidence.max(0.5);
        }
        self.frames_since_pulse = 0;
    }
}
