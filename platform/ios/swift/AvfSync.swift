import AVFoundation

/// AVFoundation side: read the sensor limits, then apply the shutter command per frame.
public enum AvfSync {
    public struct Limits {
        public var minExposureUs: Double
        public var maxExposureUs: Double
        public var supportedFps: [Double]
        public var minIso: Double
        public var maxIso: Double
    }

    /// Rolling-shutter readout time is not published by AVFoundation. Use the measured value
    /// of the device, or 0, which makes the solver treat the readout as instantaneous.
    public static func limits(from device: AVCaptureDevice, sustainUs: Double) -> Limits {
        let format = device.activeFormat
        let fps = format.videoSupportedFrameRateRanges.map {
            $0.maxFrameRate > 0 ? $0.maxFrameRate : $0.minFrameRate
        }
        return Limits(
            minExposureUs: format.minExposureDuration.seconds * 1_000_000,
            maxExposureUs: format.maxExposureDuration.seconds * 1_000_000,
            supportedFps: fps.isEmpty ? [30.0] : fps,
            minIso: format.minISO,
            maxIso: format.maxISO
        )
    }

    /// Manual exposure for one frame. The frame duration is set equal to the exposure's
    /// frame period so the strobe phase stays an integer number of cycles per frame.
    public static func apply(_ cmd: NsCommand, to device: AVCaptureDevice) {
        guard device.hasExposureModeCustom else { return }

        let exposure = CMTime(seconds: cmd.exposureUs / 1_000_000, preferredTimescale: 1_000_000)
        let frame = CMTime(seconds: 1.0 / cmd.fps, preferredTimescale: 1_000_000)
        let format = device.activeFormat

        let clampedExposure = CMTimeClampToRange(exposure, range: CMRange(
            format.minExposureDuration, format.maxExposureDuration))

        do {
            try device.lockForConfiguration()
            device.exposureMode = .custom
            device.setExposureTargetDurationAndFrameDuration(
                duration: clampedExposure, frameDuration: frame)
            device.activeVideoMinFrameDuration = frame
            device.activeVideoMaxFrameDuration = frame
            device.unlockForConfiguration()
            // AVFoundation has no direct ISO control: gain follows from the exposure and
            // frame duration the device applies.
        } catch {
            // Device busy; the next frame retries.
        }
    }
}
