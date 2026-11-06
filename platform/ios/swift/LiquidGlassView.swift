import SwiftUI

// MARK: - Liquid Glass Design System for iPhone (iOS 16+)
// Translucent frosted glass, specular edge refractions, and organic tactile controls.

/// ViewModifier providing the frosted Liquid Glass effect with specular highlight rims.
public struct LiquidGlassModifier: ViewModifier {
    var cornerRadius: CGFloat
    var opacity: Double
    var strokeOpacity: Double

    public init(cornerRadius: CGFloat = 24, opacity: Double = 0.25, strokeOpacity: Double = 0.3) {
        self.cornerRadius = cornerRadius
        self.opacity = opacity
        self.strokeOpacity = strokeOpacity
    }

    public func body(content: Content) -> some View {
        content
            .background(
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .fill(.ultraThinMaterial)
                    .opacity(opacity)
            )
            .overlay(
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .strokeBorder(
                        LinearGradient(
                            colors: [
                                Color.white.opacity(strokeOpacity),
                                Color.white.opacity(strokeOpacity * 0.15),
                                Color.white.opacity(strokeOpacity * 0.4)
                            ],
                            startPoint: .topLeading,
                            endPoint: .bottomTrailing
                        ),
                        lineWidth: 1
                    )
            )
            .shadow(color: Color.black.opacity(0.35), radius: 12, x: 0, y: 6)
    }
}

public extension View {
    func liquidGlass(cornerRadius: CGFloat = 24, opacity: Double = 0.85, strokeOpacity: Double = 0.35) -> some View {
        self.modifier(LiquidGlassModifier(cornerRadius: cornerRadius, opacity: opacity, strokeOpacity: strokeOpacity))
    }
}

// MARK: - Liquid Glass Status Pill
public struct LiquidGlassStatusPill: View {
    public let stateText: String
    public let detailText: String
    public let isLocked: Bool
    public let isScanning: Bool

    @State private var pulseAnimation = false

    public var body: some View {
        HStack(spacing: 8) {
            // Live Status Indicator Dot
            Circle()
                .fill(indicatorColor)
                .frame(width: 8, height: 8)
                .overlay(
                    Circle()
                        .stroke(indicatorColor.opacity(0.6), lineWidth: pulseAnimation && (isLocked || isScanning) ? 6 : 0)
                        .scaleEffect(pulseAnimation && (isLocked || isScanning) ? 2.0 : 1.0)
                        .opacity(pulseAnimation && (isLocked || isScanning) ? 0.0 : 0.8)
                )
                .onAppear {
                    withAnimation(.easeInOut(duration: 1.2).repeatForever(autoreverses: false)) {
                        pulseAnimation = true
                    }
                }

            VStack(alignment: .leading, spacing: 1) {
                Text(stateText.uppercased())
                    .font(.system(size: 11, weight: .bold, design: .rounded))
                    .foregroundColor(.white)
                if !detailText.isEmpty {
                    Text(detailText)
                        .font(.system(size: 10, weight: .medium, design: .monospaced))
                        .foregroundColor(.white.opacity(0.8))
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .liquidGlass(cornerRadius: 30, opacity: 0.9, strokeOpacity: 0.4)
    }

    private var indicatorColor: Color {
        if isLocked {
            return Color(red: 0.2, green: 0.85, blue: 0.55) // Emerald
        } else if isScanning {
            return Color(red: 1.0, green: 0.75, blue: 0.2) // Amber
        } else {
            return Color(red: 0.9, green: 0.3, blue: 0.3) // Red/Gray
        }
    }
}

// MARK: - Liquid Glass Lens Selector Pill
public struct LiquidGlassLensSelector: View {
    public let lenses: [Double]
    @Binding public var selectedLens: Double

    public var body: some View {
        HStack(spacing: 6) {
            ForEach(lenses, id: \.self) { lens in
                Button(action: {
                    withAnimation(.spring(response: 0.3, dampingFraction: 0.7)) {
                        selectedLens = lens
                    }
                }) {
                    Text(formatLens(lens))
                        .font(.system(size: 12, weight: selectedLens == lens ? .bold : .medium, design: .rounded))
                        .foregroundColor(selectedLens == lens ? .black : .white)
                        .frame(width: 36, height: 36)
                        .background(
                            Circle()
                                .fill(selectedLens == lens ? Color.white : Color.clear)
                        )
                }
            }
        }
        .padding(4)
        .liquidGlass(cornerRadius: 22, opacity: 0.8, strokeOpacity: 0.3)
    }

    private func formatLens(_ lens: Double) -> String {
        if lens < 1.0 {
            return String(format: ".%.0f", lens * 10)
        } else {
            return String(format: "%.0f", lens)
        }
    }
}

// MARK: - Liquid Glass Shutter Button
public struct LiquidGlassShutterButton: View {
    public let isRecording: Bool
    public let action: () -> Void

    public var body: some View {
        Button(action: action) {
            ZStack {
                // Outer Liquid Glass Ring
                Circle()
                    .strokeBorder(
                        LinearGradient(
                            colors: [
                                Color.white.opacity(0.9),
                                Color.white.opacity(0.4)
                            ],
                            startPoint: .topLeading,
                            endPoint: .bottomTrailing
                        ),
                        lineWidth: 4
                    )
                    .background(Circle().fill(.ultraThinMaterial).opacity(0.4))
                    .frame(width: 82, height: 82)
                    .shadow(color: Color.white.opacity(0.2), radius: isRecording ? 16 : 8)

                // Inner Ceramic / Recording Shape
                if isRecording {
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .fill(Color(red: 0.95, green: 0.25, blue: 0.25))
                        .frame(width: 32, height: 32)
                        .transition(.scale)
                } else {
                    Circle()
                        .fill(Color.white)
                        .frame(width: 66, height: 66)
                        .shadow(color: Color.black.opacity(0.2), radius: 4)
                        .transition(.scale)
                }
            }
        }
        .buttonStyle(PlainButtonStyle())
    }
}

// MARK: - Liquid Glass Action Button (Tools)
public struct LiquidGlassToolButton: View {
    public let systemImage: String
    public let label: String
    public let isSelected: Bool
    public let action: () -> Void

    public var body: some View {
        Button(action: action) {
            VStack(spacing: 3) {
                Image(systemName: systemImage)
                    .font(.system(size: 18, weight: .semibold))
                Text(label)
                    .font(.system(size: 9, weight: .medium, design: .rounded))
            }
            .foregroundColor(isSelected ? Color(red: 0.3, green: 0.9, blue: 0.8) : .white)
            .frame(width: 48, height: 48)
            .liquidGlass(cornerRadius: 16, opacity: isSelected ? 0.95 : 0.75, strokeOpacity: isSelected ? 0.6 : 0.25)
        }
    }
}
