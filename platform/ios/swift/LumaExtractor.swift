import AVFoundation

/// Mean luma of a video frame, normalized to 0..1.
/// Reads the Y plane of a bi-planar YUV pixel buffer (the format AVCaptureVideoDataOutput
/// produces with kCVPixelFormatType_420YpCbCr8BiPlanarFullRange).
public enum LumaExtractor {
    private static let step = 8

    public static func meanLuma(_ buffer: CVPixelBuffer) -> Double {
        CVPixelBufferLockBaseAddress(buffer, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }

        guard let base = CVPixelBufferGetBaseAddressOfPlane(buffer, 0) else { return 0.0 }
        let bytesPerRow = CVPixelBufferGetBytesPerRowOfPlane(buffer, 0)
        let height = CVPixelBufferGetHeightOfPlane(buffer, 0)
        let width = CVPixelBufferGetWidthOfPlane(buffer, 0)
        if height == 0 || width == 0 { return 0.0 }

        let y = base.bindMemory(to: UInt8.self, capacity: height * bytesPerRow)
        var sum = 0.0
        var count = 0

        var row = 0
        while row < height {
            var col = 0
            while col < width {
                sum += Double(y[row * bytesPerRow + col])
                count += 1
                col += step
            }
            row += step
        }

        if count == 0 { return 0.0 }
        return sum / (Double(count) * 255.0)
    }
}
