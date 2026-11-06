import AVFoundation

/// Vertical luminance profile of a video frame: the mean luma of each row.
///
/// The rolling shutter reads rows one after another, so the row index is a time axis.
/// PWM flicker appears as banding along this profile.
///
/// `step` subsamples rows. When `step > 1`, the profile sample rate is
/// `lineRateHz / step`, so pass that value to `PwmSyncController`.
public enum LineProfileExtractor {
    public static func rowProfile(_ buffer: CVPixelBuffer, step: Int = 1) -> [Double] {
        CVPixelBufferLockBaseAddress(buffer, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }

        guard let base = CVPixelBufferGetBaseAddressOfPlane(buffer, 0) else { return [] }
        let bytesPerRow = CVPixelBufferGetBytesPerRowOfPlane(buffer, 0)
        let height = CVPixelBufferGetHeightOfPlane(buffer, 0)
        let width = CVPixelBufferGetWidthOfPlane(buffer, 0)
        if height == 0 || width == 0 { return [] }

        let s = max(step, 1)
        let y = base.bindMemory(to: UInt8.self, capacity: height * bytesPerRow)
        var out = [Double](repeating: 0.0, count: height / s)

        for i in 0 ..< height / s {
            let row = i * s
            var sum = 0
            var count = 0
            var col = 0
            while col < width {
                sum += Int(y[row * bytesPerRow + col])
                count += 1
                col += 4
            }
            out[i] = count > 0 ? Double(sum) / (Double(count) * 255.0) : 0.0
        }
        return out
    }
}
