# Contributing

The project films concerts: the app measures LED flicker and sets the sensor exposure to a
whole number of flicker cycles, so frames and sensor rows integrate the same light. The
supported mode is **PWM flicker from 100 Hz to 5 kHz** (LED projectors, stage screens). The
strobe path (1–25 Hz) is experimental and outside the scope: do not build features on it.

## Getting set up

```sh
cargo test --workspace                                   # the platform-free core
cd platform/android
JAVA_HOME=/path/to/jdk ~/gradle-9.5.1/bin/gradle :app:assembleDebug   # the APK
```

The native library for the phone is built with `cargo-ndk`:

```sh
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/<version>
cargo ndk -t arm64-v8a -t armeabi-v7a -o platform/android/app/src/main/jniLibs \
    build -p ns-ffi --release --features android
```

`jniLibs` is build output and is not tracked. The desktop tool needs `ffmpeg` and `ffprobe` in
PATH (`cargo build --release -p ns-post`).

## Branches

- `main` — the work that is ready. The CI builds the APK from it and publishes the nightly release.
- `test-for-android` — `main` plus the debug traces and probes. **Anything under
  `crates/ns-core/examples/` goes here, not on `main`.**

Keep the branches in the same history: `test-for-android` is `main` plus its own commits. Do not
rebase one onto the other.

## Commits

- **Sign every commit** (`commit.gpgsign = true`, your own GPG key). Unsigned commits are
  rejected at review.
- **No LLM trailers.** No `Co-Authored-By:` naming an assistant, no session links, no mention of
  an LLM in the message, the code, the comments, or a file name.
- Subject: imperative, one thing, under 70 characters. Body: **why** the change, what was
  measured, what fails without it. A diff that only says "fix bug" is not reviewable.
- One commit per idea. A commit that mixes a behaviour change with a reformat makes the change
  impossible to audit.
- **Never force push** to a shared branch. History rewrites need the maintainer's confirmation.

## Before a pull request

```sh
cargo test --workspace
cd platform/android && gradle :app:testDebugUnitTest   # needs network, JUnit is not cached
```

Both must pass. Add a test for what you changed: the core is tested on synthetic signals
(`crates/ns-core/tests/`), and the pure Kotlin logic in `app/src/test/`. A change to the
controller's search, hill climb, or lock is not accepted without a test that fails on the old
behaviour.

Keep the algorithm platform-free: `crates/ns-core` has no Android or iOS imports. Device-specific
code lives in `platform/`.

## Testing on a real phone

The emulator has no `MANUAL_SENSOR` capability, so the flicker controller cannot run there.
Behaviour is only real on hardware.

```sh
adb -s <serial> install -r -g app/build/outputs/apk/debug/app-debug.apk
adb -s <serial> shell am force-stop dev.nostrobe   # required, otherwise extras are ignored
adb -s <serial> shell am start -n dev.nostrobe/.MainActivity --es sim_hz 500
adb -s <serial> logcat -d -s nostrobe:V
```

Extras: `--es sim_hz <hz>` (simulated flicker), `--ez dump_profiles true` (writes the row
profiles of 12 fixed exposures to `profiles.csv` in the app cache), `--ed luma_csv true` (one
line per analysed frame). Quote the log in the pull request: the exposure the controller asked
for, the exposure the sensor reported, the frequency it locked on.

A known flicker source is the case HDD LED driven by
`cargo run --release -p ns-core --example led_blink -- /dev/sda 500 0.5 60` (an SSD, not a hard
disk: seeks are too slow for a square wave above 100 Hz).

## Safety

- The app reduces flicker **in the video**, not in the room. Keep the photosensitive-epilepsy
  warning in the UI.
- `no-strobe-post serve` runs `ffmpeg` on what it receives. The token is mandatory off localhost,
  and it travels in clear text. Use it on a trusted network only, never expose it to the internet.
- Do not share a recorded clip of a venue without the venue's agreement.

## Reporting an issue

Include: the device model, the Android version, the log lines around the failure, the light
source (frequency if known), and the exposure the app reported. A report without the log cannot
be acted on.
