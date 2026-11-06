import AVFoundation
import SwiftUI

@main
struct AntiStrobeApp: App {
    var body: some Scene { WindowGroup { ContentView() } }
}

struct ContentView: View {
    @State private var camera = Camera()
    @State private var status = Camera.Status()
    @State private var locked = true
    @State private var denominator = 1000.0 // pose 1/1000 s
    @State private var iso: Float = 400

    var body: some View {
        ZStack {
            Preview(session: camera.session).ignoresSafeArea()
            VStack(spacing: 12) {
                Text(statusText)
                    .font(.headline.monospacedDigit())
                    .padding(10)
                    .background(.black.opacity(0.6), in: .rect(cornerRadius: 10))
                    .foregroundStyle(status.frequency == nil ? .white : .yellow)
                Spacer()
                VStack(spacing: 8) {
                    Toggle("Exposition bloquée", isOn: $locked)
                    if locked {
                        LabeledContent("Pose 1/\(Int(denominator)) s") {
                            Slider(value: $denominator, in: 240...4000, step: 10)
                        }
                        LabeledContent("ISO \(Int(iso))") {
                            Slider(value: $iso, in: status.isoRange)
                        }
                    }
                }
                .padding()
                .background(.black.opacity(0.6), in: .rect(cornerRadius: 14))
                .foregroundStyle(.white)
                Button { camera.toggleRecording() } label: {
                    Circle()
                        .fill(.red)
                        .frame(width: status.recording ? 40 : 64, height: status.recording ? 40 : 64)
                        .frame(width: 76, height: 76)
                        .overlay(Circle().stroke(.white, lineWidth: 4))
                }
                .accessibilityLabel(status.recording ? "Arrêter l'enregistrement" : "Enregistrer")
            }
            .padding()
        }
        .task {
            UIApplication.shared.isIdleTimerDisabled = true
            _ = await AVCaptureDevice.requestAccess(for: .video)
            _ = await AVCaptureDevice.requestAccess(for: .audio)
            camera.onStatus = { status = $0 }
            camera.start()
            try? await Task.sleep(for: .seconds(1)) // laisse l'exposition auto se poser avant de la convertir
            iso = camera.equivalentISO(denominator: denominator)
            applyExposure()
        }
        .onChange(of: locked) { _, on in
            if on { iso = camera.equivalentISO(denominator: denominator) }
            applyExposure()
        }
        .onChange(of: denominator) { applyExposure() }
        .onChange(of: iso) { applyExposure() }
    }

    private var statusText: String {
        let fps = "\(Int(status.fps.rounded())) i/s"
        guard let f = status.frequency else { return "Aucun strobe détecté · \(fps)" }
        return String(format: "Strobe %.1f Hz · %d %% d'images propres · %@", f, Int(status.keptRatio * 100), fps)
    }

    private func applyExposure() {
        locked ? camera.lockExposure(denominator: denominator, iso: iso) : camera.autoExposure()
    }
}

struct Preview: UIViewRepresentable {
    let session: AVCaptureSession
    final class View: UIView {
        override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
    }
    func makeUIView(context: Context) -> View {
        let v = View()
        let layer = v.layer as! AVCaptureVideoPreviewLayer
        layer.session = session
        layer.videoGravity = .resizeAspectFill
        return v
    }
    func updateUIView(_ uiView: View, context: Context) {}
}
