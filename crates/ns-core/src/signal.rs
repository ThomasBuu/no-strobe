use std::collections::VecDeque;

#[derive(Debug, Clone, Copy)]
pub struct Frame {
    /// Frame readout start, in microseconds on the camera clock.
    pub t_us: f64,
    /// Mean luma of the frame, normalized to 0..1.
    pub luminance: f64,
}

pub struct SignalBuffer {
    capacity: usize,
    frames: VecDeque<Frame>,
}

impl SignalBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            frames: VecDeque::new(),
        }
    }

    pub fn push(&mut self, frame: Frame) {
        self.frames.push_back(frame);
        while self.frames.len() > self.capacity {
            self.frames.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }

    pub fn frames(&self) -> impl Iterator<Item = &Frame> {
        self.frames.iter()
    }

    /// Median frame interval, in microseconds.
    pub fn frame_period_us(&self) -> f64 {
        if self.frames.len() < 2 {
            return 0.0;
        }
        let mut dts: Vec<f64> = self
            .frames
            .iter()
            .zip(self.frames.iter().skip(1))
            .map(|(a, b)| b.t_us - a.t_us)
            .filter(|dt| *dt > 0.0)
            .collect();
        dts.sort_by(|a, b| a.partial_cmp(b).unwrap());
        dts[dts.len() / 2]
    }

    /// Subtract a centered moving average. Removes scene brightness (camera motion, stage
    /// lighting changes) so the strobe periodicity stands out.
    pub fn detrended(&self, window: usize) -> Vec<f64> {
        detrend(&self.frames.iter().map(|f| f.luminance).collect::<Vec<f64>>(), window)
    }

    /// Times of frames whose luminance is above `threshold`. With a short exposure each
    /// strobe flash lands in at most one frame, so these are the flash times.
    pub fn pulse_times(&self, threshold: f64) -> Vec<f64> {
        self.frames
            .iter()
            .filter(|f| f.luminance > threshold)
            .map(|f| f.t_us)
            .collect()
    }

    /// Times of rising edges: the first frame above `threshold` in each run.
    /// A bright stretch spanning several frames yields one edge, not several
    /// pulses, so the cycle counts stay one per period.
    pub fn rising_edge_times(&self, threshold: f64) -> Vec<f64> {
        let mut out = Vec::new();
        let mut above = false;
        for f in self.frames.iter() {
            let now = f.luminance > threshold;
            if now && !above {
                out.push(f.t_us);
            }
            above = now;
        }
        out
    }

    pub fn times(&self) -> Vec<f64> {
        self.frames.iter().map(|f| f.t_us).collect()
    }
}

/// Subtract a centered moving average from a raw signal. Removes the slow
/// brightness changes (scene lighting, camera motion) so periodicity stands out.
pub fn detrend(x: &[f64], window: usize) -> Vec<f64> {
    let w = window.max(3) | 1;
    let half = w / 2;
    if x.len() <= 2 * half {
        return x.to_vec();
    }
    let mut out = Vec::with_capacity(x.len() - 2 * half);
    for i in half..x.len() - half {
        let mean = x[i - half..i + half + 1].iter().sum::<f64>() / w as f64;
        out.push(x[i] - mean);
    }
    out
}
