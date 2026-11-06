import AVFoundation
import CoreVideo
import Foundation

/// AVFoundation pipeline: preview + pixel-buffer analysis + recording, with the shutter driven
/// by the Rust controller.
public final class NoStrobeCamera: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    private let session = AVCaptureSession()
    private let device: AVCaptureDevice
    private let output = AVCaptureVideoDataOutput()
    private let queue = DispatchQueue(label: "nostrobe.analysis")
    private let pwmMode: Bool
    private let onStatus: (NsCommand) -> Void

    private var strobe: SyncController?
    private var pwm: PwmSyncController?
    private var last: NsCommand?

    public init(device: AVCaptureDevice, pwmMode: Bool, onStatus: @escaping (NsCommand) -> Void) {
        self.device = device
        self.pwmMode = pwmMode
        self.onStatus = onStatus
    }

    public func start() {
        session.beginUpdating()
        let l = AvfSync.limits(from: device, sustainUs: 0)
        let limits = CameraLimits(minExposureUs: l.minExposureUs, maxExposureUs: l.maxExposureUs,
                                  sustainUs: 0, supportedFps: l.supportedFps,
                                  minIso: l.minIso, maxIso: l.maxIso)
        if pwmMode {
            // AVFoundation publishes no row readout rate, so the controller sweeps exposures
            // and calibrates the flicker period from the banding metric.
            pwm = PwmSyncController(limits: limits, lineRateHz: 0)
        } else {
            strobe = SyncController(limits: limits)
        }

        do {
            let input = try AVCaptureDeviceInput(device: device)
            if session.canAddInput(input) { session.addInput(input) }
        } catch {
            session.commitConfiguration()
            return
        }

        output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String:
            kCVPixelFormatType_420YpCbCr8BiPlanarFullRange]
        output.alwaysEnablesMetadata = true
        output.setSampleBufferDelegate(self, queue: queue)
        if session.canAddOutput(output) { session.addOutput(output) }

        session.commitConfiguration()
        session.startRunning()
    }

    public func stop() {
        session.stopRunning()
        strobe = nil
        pwm = nil
        last = nil
    }

    public func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer,
                              from connection: AVCaptureConnection) {
        guard let buffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        let tUs = CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sampleBuffer)) * 1e6
        let cmd: NsCommand
        if pwmMode {
            cmd = pwm!.onProfile(profile: LineProfileExtractor.rowProfile(buffer, step: 1))
        } else {
            cmd = strobe!.onFrame(tUs: tUs, luminance: LumaExtractor.meanLuma(buffer))
        }
        onStatus(cmd)
        if needsUpdate(last, cmd) {
            last = cmd
            AvfSync.apply(cmd, to: device)
        }
    }

    private func needsUpdate(_ prev: NsCommand?, _ cmd: NsCommand) -> Bool {
        if prev == nil { return true }
        let de = abs(cmd.exposureUs - prev!.exposureUs) / prev!.exposureUs
        let di = abs(cmd.iso - prev!.iso) / prev!.iso
        return de > 0.01 || di > 0.05 || cmd.fps != prev!.fps
    }
}
