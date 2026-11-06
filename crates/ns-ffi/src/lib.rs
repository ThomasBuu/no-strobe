use ns_core::camera::CameraLimits;
use ns_core::controller::{Controller, State};
use ns_core::pwm_controller::PwmController;
use ns_core::solver::Mode;

#[cfg(feature = "android")]
mod jni;

#[repr(C)]
pub struct NsCommand {
    pub exposure_us: f64,
    pub fps: f64,
    pub iso: f64,
    /// 0 = Integrate, 1 = Gap, 2 = Flash.
    pub mode: i32,
    /// 0 = Search, 1 = Locked.
    pub state: i32,
    pub period_us: f64,
    pub confidence: f64,
    pub drift_us_per_frame: f64,
}

pub(crate) fn mode_code(mode: Mode) -> i32 {
    match mode {
        Mode::Integrate => 0,
        Mode::Gap => 1,
        Mode::Flash => 2,
        Mode::Pwm => 3,
    }
}

pub(crate) fn state_code(state: State) -> i32 {
    match state {
        State::Search => 0,
        State::Locked => 1,
    }
}

/// Create a controller. `fps_ptr` is a slice of supported frame rates.
///
/// # Safety
/// `fps_ptr` must point to `fps_len` contiguous f64 values, or be null when `fps_len` is 0.
#[no_mangle]
pub unsafe extern "C" fn ns_create(
    min_exposure_us: f64,
    max_exposure_us: f64,
    sustain_us: f64,
    fps_ptr: *const f64,
    fps_len: usize,
    min_iso: f64,
    max_iso: f64,
) -> *mut Controller {
    let fps = if fps_ptr.is_null() || fps_len == 0 {
        vec![30.0]
    } else {
        std::slice::from_raw_parts(fps_ptr, fps_len).to_vec()
    };

    let limits = CameraLimits {
        min_exposure_us,
        max_exposure_us,
        sustain_us,
        supported_fps: fps,
        min_iso,
        max_iso,
    };

    Box::into_raw(Box::new(Controller::new(limits)))
}

/// Feed one frame, get the shutter command to apply. `luminance` is the mean luma of the
/// frame, normalized to 0..1. `t_us` is the frame readout start in microseconds.
///
/// # Safety
/// `handle` must come from `ns_create` and not have been freed. `out` must be a valid pointer.
#[no_mangle]
pub unsafe extern "C" fn ns_on_frame(
    handle: *mut Controller,
    t_us: f64,
    luminance: f64,
    out: *mut NsCommand,
) {
    let cmd = (*handle).on_frame(t_us, luminance);
    *out = NsCommand {
        exposure_us: cmd.exposure_us,
        fps: cmd.fps,
        iso: cmd.iso,
        mode: mode_code(cmd.mode),
        state: state_code(cmd.state),
        period_us: cmd.period_us,
        confidence: cmd.confidence,
        drift_us_per_frame: cmd.drift_us_per_frame,
    };
}

/// Free the controller.
///
/// # Safety
/// `handle` must come from `ns_create` and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn ns_destroy(handle: *mut Controller) {
    drop(Box::from_raw(handle));
}

/// Create a PWM (LED flicker) controller. `line_rate_hz` is the sensor row
/// readout rate in rows per second; pass 0 when the device does not publish it,
/// the controller then calibrates by sweeping exposures.
///
/// # Safety
/// `fps_ptr` must point to `fps_len` contiguous f64 values, or be null when `fps_len` is 0.
#[no_mangle]
pub unsafe extern "C" fn ns_create_pwm(
    min_exposure_us: f64,
    max_exposure_us: f64,
    sustain_us: f64,
    fps_ptr: *const f64,
    fps_len: usize,
    min_iso: f64,
    max_iso: f64,
    line_rate_hz: f64,
) -> *mut PwmController {
    let fps = if fps_ptr.is_null() || fps_len == 0 {
        vec![30.0]
    } else {
        std::slice::from_raw_parts(fps_ptr, fps_len).to_vec()
    };
    let limits = CameraLimits {
        min_exposure_us,
        max_exposure_us,
        sustain_us,
        supported_fps: fps,
        min_iso,
        max_iso,
    };
    Box::into_raw(Box::new(PwmController::new(limits, line_rate_hz)))
}

/// Feed one frame's vertical luminance profile, get the shutter command.
/// `profile` is the per-row mean luma of the frame, normalized to 0..1.
///
/// # Safety
/// `handle` must come from `ns_create_pwm`. `profile` must point to `profile_len`
/// contiguous f64 values. `out` must be a valid pointer.
#[no_mangle]
pub unsafe extern "C" fn ns_on_profile(
    handle: *mut PwmController,
    profile_ptr: *const f64,
    profile_len: usize,
    out: *mut NsCommand,
) {
    let profile = std::slice::from_raw_parts(profile_ptr, profile_len);
    let cmd = (*handle).on_profile(profile);
    *out = NsCommand {
        exposure_us: cmd.exposure_us,
        fps: cmd.fps,
        iso: cmd.iso,
        mode: mode_code(cmd.mode),
        state: state_code(cmd.state),
        period_us: cmd.period_us,
        confidence: cmd.confidence,
        drift_us_per_frame: cmd.drift_us_per_frame,
    };
}

/// Free the PWM controller.
///
/// # Safety
/// `handle` must come from `ns_create_pwm` and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn ns_destroy_pwm(handle: *mut PwmController) {
    drop(Box::from_raw(handle));
}
