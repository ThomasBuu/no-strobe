import AVFoundation
import SwiftUI

/// Banc d'essai : fait clignoter la torche de l'iPhone à fréquence et rapport cyclique réglables.
@main
struct FlasherApp: App {
    var body: some Scene { WindowGroup { FlasherView() } }
}

/// Minutage dans un thread dédié, sur échéances absolues (pas de dérive).
final class Torch {
    private let lock = NSLock()
    private var frequency = 12.0, duty = 0.5, running = false
    private var toggles: [Double] = [] // instants d'allumage, pour la fréquence mesurée

    func set(frequency: Double, duty: Double) { lock.withLock { self.frequency = frequency; self.duty = duty } }

    var measured: Double? {
        lock.withLock {
            guard toggles.count > 4 else { return nil }
            return Double(toggles.count - 1) / (toggles.last! - toggles.first!)
        }
    }

    func start() {
        guard !running, let device = AVCaptureDevice.default(for: .video), device.hasTorch else { return }
        running = true
        let thread = Thread { [self] in
            guard (try? device.lockForConfiguration()) != nil else { return }
            defer { device.torchMode = .off; device.unlockForConfiguration() }
            var next = CACurrentMediaTime()
            while lock.withLock({ running }) {
                let (f, d) = lock.withLock { (frequency, duty) }
                try? device.setTorchModeOn(level: AVCaptureDevice.maxAvailableTorchLevel)
                let now = CACurrentMediaTime()
                lock.withLock {
                    toggles.append(now)
                    while let first = toggles.first, now - first > 2 { toggles.removeFirst() }
                }
                Self.sleep(until: next + d / f)
                device.torchMode = .off
                next += 1 / f
                if next < CACurrentMediaTime() { next = CACurrentMediaTime() } // retard : on ne rattrape pas en rafale
                Self.sleep(until: next)
            }
        }
        thread.qualityOfService = .userInteractive
        thread.start()
    }

    func stop() { lock.withLock { running = false; toggles = [] } }

    private static func sleep(until t: Double) {
        let dt = t - CACurrentMediaTime()
        if dt > 0 { Thread.sleep(forTimeInterval: dt) }
    }
}

struct FlasherView: View {
    @State private var torch = Torch()
    @State private var frequency = 14.3
    @State private var duty = 50.0
    @State private var running = false
    @State private var measured: Double?
    private let presets = [5, 10, 12, 14.3, 15.7, 20]
    private let refresh = Timer.publish(every: 0.5, on: .main, in: .common).autoconnect()

    var body: some View {
        Form {
            Section("Fréquence : \(frequency, specifier: "%.1f") Hz") {
                Slider(value: $frequency, in: 1...30, step: 0.1)
                HStack {
                    ForEach(presets, id: \.self) { p in
                        Button(p.formatted()) { frequency = p }
                            .buttonStyle(.bordered)
                            .font(.caption)
                    }
                }
            }
            Section("Allumé \(Int(duty)) % du cycle") {
                Slider(value: $duty, in: 10...90, step: 5)
            }
            Section {
                Button(running ? "Arrêter" : "Démarrer") {
                    running.toggle()
                    running ? torch.start() : torch.stop()
                }
                .font(.title2.bold())
                .frame(maxWidth: .infinity)
                if running {
                    LabeledContent("Fréquence mesurée", value: measured.map { String(format: "%.2f Hz", $0) } ?? "…")
                }
            } footer: {
                Text("La torche met quelques millisecondes à réagir : au-delà de ~20 Hz, la fréquence mesurée peut décrocher.")
            }
        }
        .onAppear { torch.set(frequency: frequency, duty: duty / 100); UIApplication.shared.isIdleTimerDisabled = true }
        .onChange(of: frequency) { torch.set(frequency: frequency, duty: duty / 100) }
        .onChange(of: duty) { torch.set(frequency: frequency, duty: duty / 100) }
        .onReceive(refresh) { _ in measured = torch.measured }
        .onDisappear { torch.stop() }
    }
}
