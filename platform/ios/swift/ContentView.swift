import SwiftUI
import AVFoundation

/// Main Camera Interface for iPhone with Liquid Glass design.
public struct ContentView: View {
    @State private var isRecording = false
    @State private var showGrid = false
    @State private var showEvSlider = false
    @State private var evValue: Double = 0.0
    @State private var selectedLens: Double = 1.0
    @State private var statusState: String = "Locked"
    @State private var statusDetail: String = "1000 Hz · k=2 · lead 2.1f"
    @State private var isLocked = true
    @State private var isScanning = false

    public init() {}

    public var body: some View {
        ZStack {
            // OLED Deep Black Backdrop
            Color.black.ignoresSafeArea()

            // 1. Viewfinder Area
            GeometryReader { geo in
                ZStack {
                    Rectangle()
                        .fill(Color(white: 0.05))
                        .aspectRatio(9.0 / 16.0, contentMode: .fit)
                        .clipped()

                    if showGrid {
                        GridView()
                            .aspectRatio(9.0 / 16.0, contentMode: .fit)
                    }
                }
                .frame(width: geo.size.width, height: geo.size.height)
            }

            // 2. Floating Liquid Glass Overlays
            VStack {
                // Top HUD Bar
                HStack(alignment: .top) {
                    LiquidGlassStatusPill(
                        stateText: statusState,
                        detailText: statusDetail,
                        isLocked: isLocked,
                        isScanning: isScanning
                    )
                    Spacer()

                    // Quick Tools Strip
                    VStack(spacing: 8) {
                        LiquidGlassToolButton(
                            systemImage: "grid",
                            label: "Grid",
                            isSelected: showGrid
                        ) {
                            showGrid.toggle()
                        }

                        LiquidGlassToolButton(
                            systemImage: "plusminus.circle",
                            label: "EV",
                            isSelected: showEvSlider
                        ) {
                            withAnimation(.spring(response: 0.35, dampingFraction: 0.75)) {
                                showEvSlider.toggle()
                            }
                        }

                        LiquidGlassToolButton(
                            systemImage: "wand.and.stars",
                            label: "Post",
                            isSelected: false
                        ) {
                            // Trigger post processing offload
                        }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 8)

                Spacer()

                // Optional EV Slider Overlay
                if showEvSlider {
                    HStack {
                        Text("-2.0")
                            .font(.system(size: 11, weight: .semibold, design: .monospaced))
                            .foregroundColor(.white.opacity(0.8))
                        Slider(value: $evValue, in: -2.0...2.0, step: 0.1)
                            .accentColor(Color(red: 0.3, green: 0.9, blue: 0.8))
                        Text(String(format: "%+.1f EV", evValue))
                            .font(.system(size: 11, weight: .semibold, design: .monospaced))
                            .foregroundColor(.white)
                            .frame(width: 55, alignment: .trailing)
                    }
                    .padding(.horizontal, 18)
                    .padding(.vertical, 8)
                    .liquidGlass(cornerRadius: 20, opacity: 0.9, strokeOpacity: 0.35)
                    .padding(.horizontal, 24)
                    .padding(.bottom, 6)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
                }

                // Lens Selector Floating Pill (.5x, 1x, 2x, 5x)
                LiquidGlassLensSelector(
                    lenses: [0.5, 1.0, 2.0, 5.0],
                    selectedLens: $selectedLens
                )
                .padding(.bottom, 12)

                // Bottom Liquid Glass Shutter & Control Bar
                HStack(spacing: 40) {
                    // Settings / Gallery preview
                    Button(action: {}) {
                        Image(systemName: "photo.on.rectangle.angled")
                            .font(.system(size: 20, weight: .medium))
                            .foregroundColor(.white)
                            .frame(width: 50, height: 50)
                            .liquidGlass(cornerRadius: 18, opacity: 0.8, strokeOpacity: 0.25)
                    }

                    // Main Tactile Shutter Button
                    LiquidGlassShutterButton(isRecording: isRecording) {
                        triggerHapticFeedback()
                        withAnimation(.spring(response: 0.3, dampingFraction: 0.65)) {
                            isRecording.toggle()
                        }
                    }

                    // Camera Flip
                    Button(action: {}) {
                        Image(systemName: "arrow.triangle.2.circlepath.camera")
                            .font(.system(size: 20, weight: .medium))
                            .foregroundColor(.white)
                            .frame(width: 50, height: 50)
                            .liquidGlass(cornerRadius: 18, opacity: 0.8, strokeOpacity: 0.25)
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 24)
            }
        }
    }

    private func triggerHapticFeedback() {
        #if os(iOS)
        let impact = UIImpactFeedbackGenerator(style: .medium)
        impact.impactOccurred()
        #endif
    }
}

/// Rule of Thirds Grid Overlay
struct GridView: View {
    var body: some View {
        GeometryReader { geo in
            Path { path in
                let w = geo.size.width
                let h = geo.size.height
                // Vertical lines
                path.move(to: CGPoint(x: w / 3, y: 0))
                path.addLine(to: CGPoint(x: w / 3, y: h))
                path.move(to: CGPoint(x: 2 * w / 3, y: 0))
                path.addLine(to: CGPoint(x: 2 * w / 3, y: h))
                // Horizontal lines
                path.move(to: CGPoint(x: 0, y: h / 3))
                path.addLine(to: CGPoint(x: w, y: h / 3))
                path.move(to: CGPoint(x: 0, y: 2 * h / 3))
                path.addLine(to: CGPoint(x: w, y: 2 * h / 3))
            }
            .stroke(Color.white.opacity(0.2), lineWidth: 0.75)
        }
    }
}
