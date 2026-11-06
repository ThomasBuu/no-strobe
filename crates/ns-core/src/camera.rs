/// Camera capabilities the solver needs. On Android these come from CameraCharacteristics:
/// SENSOR_INFO_EXPOSURE_TIME (max, ns), SENSOR_INFO_ROLLING_SHUTTER_SUSTAIN_DURATION (ns),
/// CONTROL_AE_TARGET_FPS_RANGE available ranges, SENSOR_INFO_SENSITIVITY_RANGE.
#[derive(Debug, Clone)]
pub struct CameraLimits {
    pub min_exposure_us: f64,
    pub max_exposure_us: f64,
    /// Rolling shutter readout time: rows are exposed over `exposure + sustain`, not `exposure`.
    pub sustain_us: f64,
    pub supported_fps: Vec<f64>,
    pub min_iso: f64,
    pub max_iso: f64,
}

impl CameraLimits {
    /// Rough numbers for a modern phone main camera at 1080p.
    pub fn typical_phone() -> Self {
        Self {
            min_exposure_us: 100.0,
            max_exposure_us: 30_000.0,
            sustain_us: 20_000.0,
            supported_fps: vec![15.0, 24.0, 25.0, 30.0, 60.0],
            min_iso: 100.0,
            max_iso: 6400.0,
        }
    }
}
