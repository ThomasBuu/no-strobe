import AVFoundation
import Photos

/// Capture à 240 i/s, analyse chaque image et enregistre deux fichiers : la version traitée (30 i/s, images prises
/// lampe éteinte) et la version brute (toutes les images, 240 i/s) pour ne rien perdre.
final class Camera: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate, AVCaptureAudioDataOutputSampleBufferDelegate {
    struct Status { var frequency: Double?; var keptRatio = 0.0; var fps = 0.0; var recording = false; var isoRange: ClosedRange<Float> = 25...3200 }

    let session = AVCaptureSession()
    var onStatus: ((Status) -> Void)?

    private let queue = DispatchQueue(label: "camera", qos: .userInitiated) // vidéo, audio et écriture sur la même file
    private var device: AVCaptureDevice?
    private let videoOut = AVCaptureVideoDataOutput()
    private let audioOut = AVCaptureAudioDataOutput()
    private var analyzer = StrobeAnalyzer(bands: 8)
    private var status = Status()

    private var processed: Recording?, raw: Recording?
    private var pacer = FramePacer<CMSampleBuffer>(interval: 1 / 30)
    private var seen = 0, kept = 0, windowStart = 0.0

    func start() {
        queue.async { [self] in
            configure()
            session.startRunning()
        }
    }

    private func configure() {
        session.beginConfiguration()
        session.sessionPreset = .inputPriority
        guard let d = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back),
              let input = try? AVCaptureDeviceInput(device: d), session.canAddInput(input) else { session.commitConfiguration(); return }
        device = d
        session.addInput(input)
        if let mic = AVCaptureDevice.default(for: .audio), let i = try? AVCaptureDeviceInput(device: mic), session.canAddInput(i) { session.addInput(i) }
        videoOut.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange]
        videoOut.setSampleBufferDelegate(self, queue: queue)
        if session.canAddOutput(videoOut) { session.addOutput(videoOut) }
        audioOut.setSampleBufferDelegate(self, queue: queue)
        if session.canAddOutput(audioOut) { session.addOutput(audioOut) }
        session.commitConfiguration()

        // le plus grand format ≤ 1080p qui monte à 240 i/s
        let fast = d.formats.filter { f in
            CMVideoFormatDescriptionGetDimensions(f.formatDescription).width <= 1920
                && f.videoSupportedFrameRateRanges.contains { $0.maxFrameRate >= 240 }
        }.max { CMVideoFormatDescriptionGetDimensions($0.formatDescription).width < CMVideoFormatDescriptionGetDimensions($1.formatDescription).width }
        guard let fmt = fast ?? d.formats.last, (try? d.lockForConfiguration()) != nil else { return }
        d.activeFormat = fmt
        let fps = fmt.videoSupportedFrameRateRanges.map(\.maxFrameRate).max() ?? 30
        d.activeVideoMinFrameDuration = CMTime(value: 1, timescale: CMTimeScale(fps))
        d.activeVideoMaxFrameDuration = CMTime(value: 1, timescale: CMTimeScale(fps))
        d.unlockForConfiguration()
        status.isoRange = fmt.minISO...fmt.maxISO
    }

    // MARK: exposition

    /// ISO qui donne la même luminosité que l'exposition auto actuelle, avec une pose de 1/denominator s.
    func equivalentISO(denominator: Double) -> Float {
        guard let d = device else { return 400 }
        let iso = d.iso * Float(d.exposureDuration.seconds * denominator)
        guard iso.isFinite, iso > 0 else { return 400 } // exposition auto pas encore posée (caméra qui démarre)
        return min(max(iso, d.activeFormat.minISO), d.activeFormat.maxISO)
    }

    func lockExposure(denominator: Double, iso: Float) {
        queue.async { [self] in
            guard let d = device, (try? d.lockForConfiguration()) != nil else { return }
            let f = d.activeFormat
            let dur = CMTimeClampToRange(CMTime(seconds: 1 / denominator, preferredTimescale: 1_000_000),
                                         range: CMTimeRange(start: f.minExposureDuration, end: f.maxExposureDuration))
            d.setExposureModeCustom(duration: dur, iso: min(max(iso, f.minISO), f.maxISO))
            d.unlockForConfiguration()
        }
    }

    func autoExposure() {
        queue.async { [self] in
            guard let d = device, (try? d.lockForConfiguration()) != nil else { return }
            d.exposureMode = .continuousAutoExposure
            d.unlockForConfiguration()
        }
    }

    // MARK: enregistrement

    func toggleRecording() { queue.async { [self] in processed == nil ? startWriting() : stopWriting() } }

    private func startWriting() {
        let video = videoOut.recommendedVideoSettingsForAssetWriter(writingTo: .mov)
        let audio = audioOut.recommendedAudioSettingsForAssetWriter(writingTo: .mov)
        guard let p = Recording(video: video, audio: audio), let r = Recording(video: video, audio: audio) else { return }
        processed = p; raw = r
        pacer = FramePacer(interval: 1 / 30)
        status.recording = true
    }

    private func stopWriting() {
        processed?.finish(); raw?.finish() // brute en second : elle apparaît juste après la traitée dans Photos
        processed = nil; raw = nil
        status.recording = false
    }

    // MARK: images

    func captureOutput(_ output: AVCaptureOutput, didOutput sb: CMSampleBuffer, from connection: AVCaptureConnection) {
        if output === audioOut {
            processed?.append(audio: sb); raw?.append(audio: sb)
            return
        }
        guard let pb = CMSampleBufferGetImageBuffer(sb) else { return }
        let pts = CMSampleBufferGetPresentationTimeStamp(sb), t = pts.seconds
        let clean = analyzer.add(time: t, bands: Self.bandLuminance(pb, bands: analyzer.bands))
        seen += 1; if clean { kept += 1 }

        raw?.append(video: sb)
        // 30 i/s régulières : pour chaque créneau, l'image propre la plus proche, horodatée au créneau (le son reste synchro)
        if let rec = processed {
            for (frame, slot) in pacer.push(time: t, clean: clean, frame: sb) {
                let time = CMTime(seconds: slot, preferredTimescale: pts.timescale)
                var timing = CMSampleTimingInfo(duration: CMTime(value: 1, timescale: 30), presentationTimeStamp: time, decodeTimeStamp: .invalid)
                var retimed: CMSampleBuffer?
                guard CMSampleBufferCreateCopyWithNewTiming(allocator: nil, sampleBuffer: frame, sampleTimingEntryCount: 1,
                                                            sampleTimingArray: &timing, sampleBufferOut: &retimed) == noErr,
                      let retimed else { continue }
                rec.append(video: retimed)
            }
        }

        if t - windowStart >= 0.25 {
            status.frequency = analyzer.frequency
            status.keptRatio = Double(kept) / Double(max(seen, 1))
            status.fps = Double(seen) / (t - windowStart)
            seen = 0; kept = 0; windowStart = t
            let s = status
            DispatchQueue.main.async { self.onStatus?(s) }
        }
    }

    /// Luminance moyenne (0…1) de bandes horizontales du capteur, une ligne sur 4 et un pixel sur 8.
    static func bandLuminance(_ pb: CVPixelBuffer, bands: Int) -> [Float] {
        CVPixelBufferLockBaseAddress(pb, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pb, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddressOfPlane(pb, 0)?.assumingMemoryBound(to: UInt8.self) else { return [Float](repeating: 0, count: bands) }
        let w = CVPixelBufferGetWidthOfPlane(pb, 0), h = CVPixelBufferGetHeightOfPlane(pb, 0), rowBytes = CVPixelBufferGetBytesPerRowOfPlane(pb, 0)
        return (0..<bands).map { b in
            var sum = 0, n = 0
            for y in stride(from: b * h / bands, to: (b + 1) * h / bands, by: 4) {
                let row = base + y * rowBytes
                for x in stride(from: 0, to: w, by: 8) { sum += Int(row[x]); n += 1 }
            }
            return Float(sum) / Float(max(n, 1)) / 255
        }
    }
}

/// Un fichier vidéo + son en cours d'écriture, enregistré dans Photos à la fin.
private final class Recording {
    private let writer: AVAssetWriter, videoIn: AVAssetWriterInput, audioIn: AVAssetWriterInput
    private var started = false

    init?(video: [String: Any]?, audio: [String: Any]?) {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".mov")
        guard let w = try? AVAssetWriter(outputURL: url, fileType: .mov) else { return nil }
        writer = w
        videoIn = AVAssetWriterInput(mediaType: .video, outputSettings: video)
        videoIn.expectsMediaDataInRealTime = true
        videoIn.transform = CGAffineTransform(rotationAngle: .pi / 2) // capteur en paysage, appli en portrait
        audioIn = AVAssetWriterInput(mediaType: .audio, outputSettings: audio)
        audioIn.expectsMediaDataInRealTime = true
        if w.canAdd(videoIn) { w.add(videoIn) }
        if w.canAdd(audioIn) { w.add(audioIn) }
        guard w.startWriting() else { return nil }
    }

    func append(video sb: CMSampleBuffer) {
        guard videoIn.isReadyForMoreMediaData else { return }
        if !started { writer.startSession(atSourceTime: CMSampleBufferGetPresentationTimeStamp(sb)); started = true }
        videoIn.append(sb)
    }

    func append(audio sb: CMSampleBuffer) {
        if started, audioIn.isReadyForMoreMediaData { audioIn.append(sb) }
    }

    func finish() {
        guard started else { writer.cancelWriting(); return }
        videoIn.markAsFinished(); audioIn.markAsFinished()
        let w = writer
        w.finishWriting {
            PHPhotoLibrary.shared().performChanges({
                PHAssetCreationRequest.creationRequestForAssetFromVideo(atFileURL: w.outputURL)
            }) { _, _ in try? FileManager.default.removeItem(at: w.outputURL) }
        }
    }
}
