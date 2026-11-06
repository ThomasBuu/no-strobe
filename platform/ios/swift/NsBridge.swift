import Foundation

/// Swift wrapper over the C API in `include/no_strobe.h`.
/// Add that header to the bridging header of the target.
public enum NsMode: Int {
    case integrate = 0
    case gap = 1
    case flash = 2
    case pwm = 3
}

public enum NsState: Int {
    case search = 0
    case locked = 1
}

public struct NsCommand {
    public var exposureUs: Double
    public var fps: Double
    public var iso: Double
    public var mode: NsMode
    public var state: NsState
    public var periodUs: Double
    public var confidence: Double
    public var driftUsPerFrame: Double
}

public struct CameraLimits {
    public var minExposureUs: Double
    public var maxExposureUs: Double
    public var sustainUs: Double
    public var supportedFps: [Double]
    public var minIso: Double
    public var maxIso: Double

    public init(minExposureUs: Double, maxExposureUs: Double, sustainUs: Double,
                supportedFps: [Double], minIso: Double, maxIso: Double) {
        self.minExposureUs = minExposureUs
        self.maxExposureUs = maxExposureUs
        self.sustainUs = sustainUs
        self.supportedFps = supportedFps
        self.minIso = minIso
        self.maxIso = maxIso
    }
}

public final class SyncController {
    private var handle: OpaquePointer?

    public init(limits: CameraLimits) {
        handle = limits.supportedFps.withUnsafeBufferPointer { buf in
            ns_create(limits.minExposureUs, limits.maxExposureUs, limits.sustainUs,
                      buf.baseAddress, buf.count, limits.minIso, limits.maxIso)
        }
    }

    /// Feed one frame (readout start in microseconds, mean luma 0..1), get the shutter command.
    public func onFrame(tUs: Double, luminance: Double) -> NsCommand {
        var raw = NsCommand(exposure_us: 0, fps: 0, iso: 0, mode: 0, state: 0,
                            period_us: 0, confidence: 0, drift_us_per_frame: 0)
        ns_on_frame(handle, tUs, luminance, &raw)
        return NsCommand(
            exposureUs: raw.exposure_us,
            fps: raw.fps,
            iso: raw.iso,
            mode: NsMode(rawValue: Int(raw.mode)) ?? .integrate,
            state: NsState(rawValue: Int(raw.state)) ?? .search,
            periodUs: raw.period_us,
            confidence: raw.confidence,
            driftUsPerFrame: raw.drift_us_per_frame
        )
    }

    deinit {
        if handle != nil {
            ns_destroy(handle)
            handle = nil
        }
    }
}

/// PWM (LED flicker) controller.
///
/// AVFoundation does not publish the sensor row readout rate, so pass
/// `lineRateHz = 0` and let the controller calibrate by sweeping exposures.
public final class PwmSyncController {
    private var handle: OpaquePointer?

    public init(limits: CameraLimits, lineRateHz: Double) {
        handle = limits.supportedFps.withUnsafeBufferPointer { buf in
            ns_create_pwm(limits.minExposureUs, limits.maxExposureUs, limits.sustainUs,
                          buf.baseAddress, buf.count, limits.minIso, limits.maxIso, lineRateHz)
        }
    }

    /// Feed one frame's vertical luminance profile (per-row mean luma, 0..1).
    public func onProfile(profile: [Double]) -> NsCommand {
        var raw = NsCommand(exposure_us: 0, fps: 0, iso: 0, mode: 0, state: 0,
                            period_us: 0, confidence: 0, drift_us_per_frame: 0)
        profile.withUnsafeBufferPointer { buf in
            ns_on_profile(handle, buf.baseAddress, buf.count, &raw)
        }
        return NsCommand(
            exposureUs: raw.exposure_us,
            fps: raw.fps,
            iso: raw.iso,
            mode: NsMode(rawValue: Int(raw.mode)) ?? .pwm,
            state: NsState(rawValue: Int(raw.state)) ?? .search,
            periodUs: raw.period_us,
            confidence: raw.confidence,
            driftUsPerFrame: raw.drift_us_per_frame
        )
    }

    /// Sets the hardware pipeline depth in frames for lead prediction.
    public func setPipelineDepth(_ depth: Double) {
        if let h = handle {
            ns_set_pipeline_depth(h, depth)
        }
    }

    /// Compute per-row gains to flatten rolling-shutter banding in one frame.
    public static func rowGain(profile: [Double], lineRateHz: Double, maxGain: Double = 3.0) -> (gains: [Double], fHz: Double) {
        var out = [Double](repeating: 1.0, count: profile.count)
        let fHz = profile.withUnsafeBufferPointer { pBuf in
            out.withUnsafeMutableBufferPointer { oBuf in
                ns_row_gain(pBuf.baseAddress, pBuf.count, lineRateHz, maxGain, oBuf.baseAddress)
            }
        }
        return (out, fHz)
    }

    deinit {
        if handle != nil {
            ns_destroy_pwm(handle)
            handle = nil
        }
    }
}
