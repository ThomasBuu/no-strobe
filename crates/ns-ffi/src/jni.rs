use jni::objects::{JClass, JDoubleArray, ReleaseMode};
use jni::sys::{jdouble, jlong};
use jni::JNIEnv;

use ns_core::camera::CameraLimits;
use ns_core::controller::Controller;
use ns_core::pwm_controller::PwmController;

use crate::{mode_code, state_code};

/// Output slot order for `nativeOnFrame`:
/// [0] exposure_us, [1] fps, [2] iso, [3] mode (0 Integrate, 1 Gap, 2 Flash),
/// [4] state (0 Search, 1 Locked), [5] period_us, [6] confidence, [7] drift_us_per_frame.
const OUT_LEN: usize = 8;

#[no_mangle]
pub extern "system" fn Java_dev_nostrobe_NsJni_nativeCreate(
    mut env: JNIEnv,
    _class: JClass,
    min_exposure_us: jdouble,
    max_exposure_us: jdouble,
    sustain_us: jdouble,
    fps: JDoubleArray,
    min_iso: jdouble,
    max_iso: jdouble,
) -> jlong {
    let fps_vec = unsafe {
        env.get_array_elements(&fps, ReleaseMode::CopyBack)
            .map(|arr| arr.iter().map(|v| *v).collect::<Vec<f64>>())
            .unwrap_or_default()
    };

    let limits = CameraLimits {
        min_exposure_us,
        max_exposure_us,
        sustain_us,
        supported_fps: if fps_vec.is_empty() { vec![30.0] } else { fps_vec },
        min_iso,
        max_iso,
    };

    let handle = Box::into_raw(Box::new(Controller::new(limits)));
    handle as jlong
}

#[no_mangle]
pub extern "system" fn Java_dev_nostrobe_NsJni_nativeOnFrame(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    t_us: jdouble,
    luminance: jdouble,
    out: JDoubleArray,
) {
    let ctrl = unsafe { &mut *(handle as *mut Controller) };
    let cmd = ctrl.on_frame(t_us, luminance);

    let values: [f64; OUT_LEN] = [
        cmd.exposure_us,
        cmd.fps,
        cmd.iso,
        mode_code(cmd.mode) as f64,
        state_code(cmd.state) as f64,
        cmd.period_us,
        cmd.confidence,
        cmd.drift_us_per_frame,
    ];

    let _ = env.set_double_array_region(&out, 0, &values);
}

#[no_mangle]
pub extern "system" fn Java_dev_nostrobe_NsJni_nativeDestroy(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if handle != 0 {
        unsafe { drop(Box::from_raw(handle as *mut Controller)) };
    }
}

/// PWM controller handle. `line_rate_hz` is the sensor row readout rate; pass 0
/// when the device does not publish it.
#[no_mangle]
pub extern "system" fn Java_dev_nostrobe_NsJni_nativeCreatePwm(
    mut env: JNIEnv,
    _class: JClass,
    min_exposure_us: jdouble,
    max_exposure_us: jdouble,
    sustain_us: jdouble,
    fps: JDoubleArray,
    min_iso: jdouble,
    max_iso: jdouble,
    line_rate_hz: jdouble,
) -> jlong {
    let fps_vec = unsafe {
        env.get_array_elements(&fps, ReleaseMode::CopyBack)
            .map(|arr| arr.iter().map(|v| *v).collect::<Vec<f64>>())
            .unwrap_or_default()
    };
    let limits = CameraLimits {
        min_exposure_us,
        max_exposure_us,
        sustain_us,
        supported_fps: if fps_vec.is_empty() { vec![30.0] } else { fps_vec },
        min_iso,
        max_iso,
    };
    let handle = Box::into_raw(Box::new(PwmController::new(limits, line_rate_hz)));
    handle as jlong
}

/// Feed the per-row mean luma of a frame, get the command in `out`.
#[no_mangle]
pub extern "system" fn Java_dev_nostrobe_NsJni_nativeOnProfile(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    profile: JDoubleArray,
    out: JDoubleArray,
) {
    let ctrl = unsafe { &mut *(handle as *mut PwmController) };
    let values = unsafe {
        env.get_array_elements(&profile, ReleaseMode::CopyBack)
            .map(|arr| arr.iter().map(|v| *v).collect::<Vec<f64>>())
            .unwrap_or_default()
    };
    let cmd = ctrl.on_profile(&values);
    let out_values: [f64; OUT_LEN] = [
        cmd.exposure_us,
        cmd.fps,
        cmd.iso,
        mode_code(cmd.mode) as f64,
        state_code(cmd.state) as f64,
        cmd.period_us,
        cmd.confidence,
        cmd.drift_us_per_frame,
    ];
    let _ = env.set_double_array_region(&out, 0, &out_values);
}

#[no_mangle]
pub extern "system" fn Java_dev_nostrobe_NsJni_nativeDestroyPwm(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if handle != 0 {
        unsafe { drop(Box::from_raw(handle as *mut PwmController)) };
    }
}

/// Test only: the per-row luminance profile a PWM light would give (see
/// `ns_core::pwm::synthetic_profile`). The app's simulated-flicker mode uses it to feed the
/// closed loop with a flicker that reacts to the exposure the sensor really applied.
#[no_mangle]
pub extern "system" fn Java_dev_nostrobe_NsJni_nativeSyntheticProfile<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass,
    exposure_us: jdouble,
    f_hz: jdouble,
    phase_cycles: jdouble,
    duty: jdouble,
    line_rate_hz: jdouble,
    rows: jni::sys::jint,
) -> jni::objects::JDoubleArray<'a> {
    let rows = rows.max(0) as usize;
    let p = ns_core::pwm::synthetic_profile(exposure_us, f_hz, phase_cycles, duty, line_rate_hz, rows);
    let arr = env.new_double_array(rows as jni::sys::jsize).expect("new_double_array");
    let _ = env.set_double_array_region(&arr, 0, &p);
    arr
}
