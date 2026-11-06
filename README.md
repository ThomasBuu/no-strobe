# no-strobe

Camera app (Android + iOS) that films concerts without LED flicker. It measures the flicker
of LED projectors and stage screens, then sets the sensor exposure to a whole number of
flicker cycles (`exposure = k/f`), so every frame and every sensor row integrates the same
amount of light: no banding, no brightness pumping.

**Scope: LED / PWM flicker, 100 Hz–5 kHz.** This is the supported mode.

| Mode | Light source | Frequency | Status |
|---|---|---|---|
| PWM | LED projectors, stage screens | 100 Hz–5 kHz | **main target**, real-time on Android and iOS |
| Strobe | deliberate slow flashes | 1–25 Hz | experimental, outside the project scope (see *Strobe modes*) |

A desktop post-processing tool fixes clips recorded without sync (see *Post-processing*).

**Status:** the Rust core is tested on synthetic signals only (`cargo test --workspace`).
Nothing has been validated on a real phone yet: row readout time varies a lot between
sensors, and some Android devices ignore manual exposure in video. See *Roadmap*.

## Why exposure sync works

- **Flicker** comes from frames integrating a different number of cycles. If the exposure
  window is an exact integer multiple of the period `T`, every frame integrates the same
  amount of light → no flicker, and the phase no longer matters.
- **Rolling-shutter banding** comes from rows being read out at different times. A row is
  exposed over `exposure + sustain`, where `sustain` is the sensor readout duration
  (`SENSOR_INFO_ROLLING_SHUTTER_SUSTAIN_DURATION` on Android, unpublished on iOS). If that
  window spans several cycles, top and bottom of the frame see different phases → horizontal
  bands. Exposure = `k/f` removes them.
- **Phase drift**: the frame period `F = 1/fps` is rarely a multiple of `T`. Each frame the
  phase shifts by `F - round(F/T)·T`. The tracker watches that drift and adapts.
- Accuracy matters: 1 % error on `f` leaves visible pumping. The PWM path refines the estimate
  in a closed loop, measuring residual banding at candidate exposures.

### Strobe modes

> Experimental and outside the project scope (LED flicker only). Kept because the code exists
> and is tested, but it is not the target of the app. Exposure sync alone cannot flatten a
> slow strobe when `exposure = k·T` exceeds the frame period (e.g. 10 Hz at 30 fps).

| Mode | Exposure | Effect |
|---|---|---|
| `Integrate` | `k·T` | strobe energy uniform across frames, image stays bright |
| `Gap` | dark interval − sustain | strobe fully removed from the image |
| `Flash` | one flash window | every frame captures exactly one flash |

`Gap` is what you want for a subject being blinded by a strobe. `Integrate` is what you want
when the strobe is part of the show. The solver picks per period, and re-picks when the period
drifts.

### PWM mode

Frame-mean luma cannot resolve above `fps/2` — at 30 fps it aliases everything past 15 Hz. So
the PWM path reads the **per-row luminance profile** of a frame: the rolling shutter samples
light at the row readout rate (20–100 kHz), so a 100 Hz–5 kHz flicker shows up as a vertical
banding pattern.

1. `LineProfileExtractor` builds a per-row mean luma array from the Y plane.
2. `estimate_f` autocorrelates the detrended profile. FFT bins are too coarse (bin spacing
   `line_rate/N` ≈ 78 Hz), so the lag is refined parabolically.
3. Trials at `k/f` are measured for residual banding; the lowest wins, ISO compensates the
   light the shorter exposure loses.
4. When the sensor's row readout rate is unknown (iOS), the controller hill-climbs on banding
   amplitude directly: it finds `e = k/f` without knowing `f`, then derives `f = 1/e`.
5. Every 30 frames, and whenever residual banding grows, the controller re-estimates: LED
   drivers change duty cycle and frequency mid-show.

## Layout

```
Cargo.toml             workspace: ns-core, ns-ffi, ns-post
cbindgen.toml          C header generation
crates/ns-core/        platform-free algorithm (no deps)
  src/lib.rs         module list, the public API of the crate
  src/camera.rs        sensor limits (exposure range, rolling-shutter sustain, fps, iso)
  src/signal.rs        frame ring buffer, detrend, pulse detection
  src/detector.rs      autocorrelation period search, pulse phase fit, drift tracking
  src/solver.rs        candidate exposure/fps modes + scoring
  src/controller.rs    strobe path: Search → Locked, per-frame Command
  src/pwm.rs           autocorrelation frequency estimate, banding metrics, synthetic profile
  src/pwm_controller.rs PWM path: Search → Hill → Locked → Recalib
  src/restorer.rs      post-processing: per-frame gain, per-row gain
  examples/            13 traces and probes: demo, pwm_demo, sweep_demo, deflicker,
                       strobe_frames, gen_frames, diag, blind_trace, drift_trace,
                       lag_probe, metric_probe, pulse_probe, led_blink
  tests/               sync.rs, pwm.rs, restorer.rs
crates/ns-ffi/         C API (cbindgen header) + JNI for Android
crates/ns-post/        no-strobe-post: fix.rs (clip fix, streaming ffmpeg pipes),
                       serve.rs (HTTP server the phone offloads to), main.rs (CLI)
include/no_strobe.h    generated C header
platform/android/      Gradle app (settings.gradle, app/build.gradle)
  app/src/main/java/dev/nostrobe/
    NsJni.kt                JNI bridge, SyncController, PwmSyncController
    LumaExtractor.kt        mean luma of a YUV_420_888 frame
    LineProfileExtractor.kt per-row luma profile + row readout rate
    Camera2Sync.kt          CameraCharacteristics limits + per-frame CaptureRequest
    NoStrobeCamera.kt       capture session, per-frame loop, high-speed session,
                            profile dump for offline study
    MainActivity.kt         preview, status line, record button, settings, gallery
    AspectFrameLayout.kt    viewfinder box that keeps the frame's own aspect ratio
    LensOptions.kt          lens chips: pin one lens of a logical multi-camera
    GridOverlay.kt          rule-of-thirds grid drawn over the preview
    OnDevicePost.kt         fix a clip on the phone (MediaCodec, temporal gain only)
    DesktopPost.kt          POST a clip to no-strobe-post serve
  app/src/main/res/
    layout/main.xml         phone layout
    layout-w700dp/main.xml  wide layout (foldable opened flat, tablet): viewfinder + settings
    drawable/               capture button (idle, recording), focus ring
  app/src/main/jniLibs/<abi>/libns_ffi.so   build output, not tracked (arm64-v8a, armeabi-v7a)
platform/ios/swift/
  NsBridge.swift          C API wrapper (bridging header)
  LumaExtractor.swift     mean luma of a YUV pixel buffer
  LineProfileExtractor.swift per-row luma profile
  AvfSync.swift           AVCaptureDevice limits + per-frame exposure
  NoStrobeCamera.swift    capture session, per-frame loop, re-apply on change
tools/
  screen_flicker.py       white/black alternation at a set Hz (Tkinter)
  make_flicker_video.py   flicker clip through ffmpeg, prints the rate it produces
  adb_tap.py              tap an Android control by text/content-desc (uiautomator)
  postprocess.sh          script-only clip fix (ffmpeg + the deflicker example)
```

## Per-frame loop

Both platforms do the same thing:

1. Get the frame readout start time (`SENSOR_TIMESTAMP` on Android,
   `AVCaptureConnection` timestamp on iOS) and either the mean luma of the Y plane (strobe)
   or the per-row luma profile (PWM).
2. `SyncController.onFrame(t_us, luminance)` or `PwmSyncController.onProfile(profile)` →
   `Command`.
3. Apply `Command` to the next capture request / device configuration.

During `Search` the strobe controller forces a short exposure (8 ms): a long exposure
integrates whole cycles and flattens the flicker signal. After ~45 frames it autocorrelates
the detrended luma over 4–40 Hz, fits the pulse phase, and locks when confidence ≥ 0.45.

The fit uses **rising edges** (the first frame above the threshold in each run), not every
frame above the threshold. A bright strobe flash spans several frames at 60 fps, so counting
pulses gives 3 edges per cycle and the integer cycle counts collapse — the fit landed 3 % off.
Edges give one point per cycle: on a 10 Hz / 60 fps clip, 240 frames → 40 edges → T = 100000 µs
exact, confidence 1.00.

## Test sources

No concert needed to validate the algorithm:

- **PC screen** (`tools/screen_flicker.py`, Tkinter, white/black alternation at a set Hz).
  A screen refreshes at 60 Hz, so it covers the strobe band (1–25 Hz) only. A screen cannot
  produce the 100 Hz–5 kHz PWM band.
- **Recorded flicker video** (`tools/make_flicker_video.py` → ffmpeg). The rate is quantized to
  whole frames, so the script prints the rate it actually produces and the percent error.
  `crates/ns-core/examples/strobe_frames.rs` runs the strobe controller over the extracted PGM
  frames.
- **Third-party phone flashes**: several phones flashing in the same room give multiple strobes
  at unrelated periods. The autocorrelation picks the strongest; the second estimate is on the
  roadmap.
- **Arduino/ESP32 PWM LED** for the PWM band (100 Hz–5 kHz), the only source that reaches the
  frequencies LED stage projectors use. Also the reference for calibrating `sustain_us` and the
  row readout rate per device.

The PWM controller searches at the sensor's minimum exposure for the same reason: at 1 kHz, a
1000 µs exposure integrates a whole cycle and the profile goes flat.

## Android

Requirements: Camera2 `MANUAL_SENSOR` capability
(`REQUEST_AVAILABLE_CAPABILITIES_MANUAL_SENSOR`), `SENSOR_INFO_EXPOSURE_TIME`,
`SENSOR_INFO_ROLLING_SHUTTER_SUSTAIN_DURATION` (API 31+; absent devices get sustain = 0).

Build the native library:

```sh
cargo install cargo-ndk
rustup target add aarch64-linux-android armv7-linux-androideabi
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/<version>
cargo ndk -t arm64-v8a -t armeabi-v7a -o platform/android/app/src/main/jniLibs \
    build -p ns-ffi --release --features android
# produces libns_ffi.so for each ABI in platform/android/app/src/main/jniLibs/<abi>/
# (arm64-v8a: current phones, armeabi-v7a: 32-bit ARM devices; no x86 build, phones only)
```

Then the APK (`platform/android`, Gradle 9.5.1 + AGP 9.3.0, verified in this repo):

```sh
JAVA_HOME=/path/to/jdk ANDROID_HOME=/path/to/android-sdk gradle :app:assembleDebug
# app/build/outputs/apk/debug/app-debug.apk
```

Notes verified on the real build:
- AGP 9 has built-in Kotlin support. Applying `org.jetbrains.kotlin.android` fails the build
  (issuetracker 438678642).
- `build.gradle` is Groovy DSL: `ndk { abiFilters "arm64-v8a", "armeabi-v7a" }`. Kotlin syntax
  (`abiFilters = listOf(...)`) fails — `listOf` is not a method of `NdkOptions`.
- Gradle 9.1.0+ is required to run on Java 25; AGP 9.3 needs Gradle ≥ 9.5.0.
- `local.properties` holds `sdk.dir=/path/to/android-sdk`.

`NsJni.kt` loads `libns_ffi.so` (`System.loadLibrary("ns_ffi")`). The JNI symbols are
`Java_dev_nostrobe_NsJni_nativeCreate`, `nativeOnFrame`, `nativeDestroy`,
`nativeCreatePwm`, `nativeOnProfile`, `nativeDestroyPwm` in `crates/ns-ffi/src/jni.rs`.

In the capture session:

```kotlin
val chars = manager.getCameraCharacteristics(cameraId)
require(Camera2Sync.supportsManualSensor(chars))
val limits = Camera2Sync.limitsFromCharacteristics(chars)

// strobe path
val strobe = NsJni.create(limits)
// PWM path
val pwm = NsJni.createPwm(limits, LineProfileExtractor.lineRate(height, sustainNs))

// in the ImageReader / YUV callback:
val tUs = image.timestamp.toDouble() / 1000.0          // SENSOR_TIMESTAMP is nanoseconds
val cmd = strobe.onFrame(tUs, LumaExtractor.meanLuma(image))
val cmd2 = pwm.onProfile(LineProfileExtractor.rowProfile(image))
// apply to the next CaptureRequest.Builder via Camera2Sync.apply
```

When the row readout rate is not published, pass `0.0` and the controller hill-climbs.
`CONTROL_AE_MODE_OFF` and `CONTROL_CAPTURE_INTENT_MANUAL` are required, otherwise the 3A
routine overwrites `SENSOR_EXPOSURE_TIME` every frame.

## iOS

Build the static library and link it into the app target:

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo build -p ns-ffi --release --target aarch64-apple-ios
# target/aarch64-apple-ios/release/libns_ffi.a + include/no_strobe.h
```

Add `include/no_strobe.h` to the bridging header, link `libns_ffi.a`, then:

```swift
let device = /* your AVCaptureDevice */
let limits = AvfSync.limits(from: device, sustainUs: 0)
let pwm = PwmSyncController(limits: limits, lineRateHz: 0)

// in the video data output delegate:
let tUs = CMSampleBufferGetSamplePresentationTime(sample).seconds * 1_000_000
let cmd = pwm.onProfile(LineProfileExtractor.rowProfile(pixelBuffer))
AvfSync.apply(cmd, to: device)
```

AVFoundation exposes no rolling-shutter sustain and no direct ISO control. Exposure time and
frame duration are the controls; gain follows from them.

## Post-processing

Fixes a clip recorded without sync. Two artefacts, two gains, in `crates/ns-core/src/restorer.rs`:

- **Temporal flicker**: each frame's brightness depends on how much of the flash its exposure
  window caught. `temporal_gain(means, window, max_gain)` divides each frame by a local moving
  mean of its own mean luma.
- **Banding**: a rolling-shutter frame samples the flicker row by row. `row_gain(profile,
  line_rate_hz, max_gain)` estimates `f` from the profile, averages the profile over all rows
  sharing a phase within one period (that average is the waveform), and divides each row by the
  waveform normalized to mean 1. Subtracting a moving average over one period averages the wave
  to zero, so it leaves the profile unchanged — the phase average is what works.

The CLI works on ffmpeg-extracted frames:

```sh
ffmpeg -i clip.mp4 frames/frame_%05d.pgm
cargo run -p ns-core --example deflicker frames/ out/ 40000   # row readout rate, Hz
ffmpeg -framerate 30 -i out/frame_%05d.pgm -c:v libx264 fixed.mp4
```

`line_rate_hz` is the row readout rate. Pass 0 to only flatten temporal flicker. The gain is
clamped (default 4×): rows the sensor never exposed cannot be recovered.

## Tests

```sh
cargo test --workspace
cargo run -p ns-core --example demo 100000 30      # strobe: 10 Hz at 30 fps
cargo run -p ns-core --example pwm_demo 2000 40000 # PWM: 2 kHz, row rate 40 kHz
cargo run -p ns-core --example sweep_demo          # PWM with unknown row rate
```

`tests/sync.rs` synthesizes strobe frames and checks: 10 Hz detection within 2 %, exposure =
integer multiple of `T`, `Gap`/`Flash` fallback for a fast strobe, and
`exposure + sustain ≤ frame period` for every period from 5 to 30 Hz.

`tests/pwm.rs` synthesizes per-row profiles (square wave integrated over each row's exposure
window) and checks: 500 Hz and 1 kHz detection within 1 %, lock on an integer multiple of the
period, re-lock when the frequency drifts 500 → 512 Hz, no lock on a static scene, and the
sweep path finding the frequency without a row rate.

`tests/restorer.rs` checks: temporal gain flattens a 10 Hz strobe's frame-to-frame brightness,
row gain flattens banding at 1 kHz, and both stay at 1.0 on a clean clip. Banding is only
readable when the exposure window is shorter than one flicker cycle, so the test uses 100 µs.

## Roadmap

1. **Real-device validation** (first priority): film a PWM LED of known frequency
   (Arduino/ESP32, 200 Hz–2 kHz) on a few Android and iOS phones, and measure the residual
   banding and pumping before/after.
2. **Calibration**: measure `sustain_us` and the row readout rate per device by filming that
   known source and fitting the banding slope.
3. **Android manual exposure**: read back `CaptureResult` to check the requested exposure is
   really applied, and tell the user when the device ignores it.
4. **Colour**: the restorer works on the Y plane only. Extending to RGB needs the gain applied
   per channel, or the flicker is colour-tinted at the bands.
5. **Outside the scope for now** (strobe path): multi-strobe venues, beat-synced drift,
   torch/flash sync.

## Known limits

- **PWM path:** a 1 % error on `f` leaves visible pumping, so the closed loop refines the
  estimate; exposure resolution is limited by the sensor row time, which hurts towards 1 kHz+.
  Phone sensors differ widely; nothing here is validated on hardware yet.
- The strobe detector (experimental) needs a visible periodic signal. A strobe behind the subject, or heavy haze,
  lowers contrast; the confidence gate keeps the app in `Search` and the image stays at short
  exposure.
- Exposure range is device-bound. A phone with `max_exposure_us` below `T` cannot use
  `Integrate` for that `T`; the solver falls back to `Gap`/`Flash`.
- `Gap` mode throws away most light, so ISO climbs and the image gets noisy.
- `CONTROL_AE_TARGET_FPS_RANGE` is a Range<Int>; fractional fps from the solver is rounded.
  Devices that only accept integer fps lose the fine drift correction.
- Row readout rate is unpublished on iOS, so the PWM path hill-climbs; it costs ~90 frames
  before lock.
- The app reduces flicker **in the video**, not in the room. Warn users about photosensitive
  epilepsy.

## Desktop post-processing and offload

`no-strobe-post` (crate `ns-post`, needs `ffmpeg` and `ffprobe` in PATH) fixes a clip on a
computer, streaming the frames through ffmpeg so memory stays small. It de-flickers the luma
with the restorer, and keeps the original chroma, audio, frame timing and rotation.

```sh
cargo build --release -p ns-post
target/release/no-strobe-post fix clip.mp4 [--line-rate HZ] [-o clip-fixed.mp4]
```

For the phone to offload a clip to the computer, run the server:

```sh
export NO_STROBE_TOKEN=some-secret
target/release/no-strobe-post serve --listen 0.0.0.0:8765   # --token is required off localhost
```

`POST /fix?line_rate=<hz>` with the mp4 as body (and `Authorization: Bearer <token>`) answers
with the fixed mp4; `GET /health` answers `ok`. One clip is processed at a time. The server
runs ffmpeg on what it receives and the token travels in clear text (plain HTTP): use it on a
trusted network, or behind a VPN/tunnel such as Tailscale. `tools/postprocess.sh` remains as
a script-only alternative.

On the phone, the POST button offers "Fix on this phone" (decode, scale the luma by the
temporal gain, re-encode with MediaCodec; no network, no banding gain, slower than a desktop)
and "Send to the desktop and fix" (upload to `no-strobe-post serve`, address and token in the
settings dialog). Both save the result next to the original in Movies/no-strobe.

Limit: the gain is computed from each frame's own brightness, so a genuine fade or an
auto-exposure change in the clip is flattened too.
