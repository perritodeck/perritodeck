import PerritoKit
import SwiftUI

/// The Android app's dark palette (android/.../ui/Theme.kt).
enum Theme {
    static let background = Color(rgb: 0x11111B)
    static let surface = Color(rgb: 0x1E1E2E)
    static let brand = Color(rgb: 0xFAB387)
    static let onSurface = Color(rgb: 0xCDD6F4)
    static let muted = Color(rgb: 0x9399B2)
    static let error = Color(rgb: 0xF38BA8)
    static let ok = Color(rgb: 0xA6E3A1)
    static let warning = Color(rgb: 0xF9E2AF)
}

extension Color {
    init(rgb: Int) {
        self.init(
            red: Double((rgb >> 16) & 0xFF) / 255,
            green: Double((rgb >> 8) & 0xFF) / 255,
            blue: Double(rgb & 0xFF) / 255
        )
    }

    /// `#RRGGBB` from the host, or nil if malformed.
    init?(hex: String) {
        guard let c = RGBColor(hex: hex) else { return nil }
        self.init(red: Double(c.red) / 255, green: Double(c.green) / 255, blue: Double(c.blue) / 255)
    }
}

/// Filled brand button (Material's Button).
struct PrimaryButtonStyle: ButtonStyle {
    var fullWidth = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.body.weight(.semibold))
            .foregroundColor(Theme.background)
            .padding(.horizontal, 20)
            .padding(.vertical, 12)
            .frame(maxWidth: fullWidth ? .infinity : nil)
            .background(Capsule().fill(Theme.brand.opacity(configuration.isPressed ? 0.75 : 1)))
            .contentShape(Capsule())
    }
}

/// Outlined button (Material's OutlinedButton).
struct OutlineButtonStyle: ButtonStyle {
    var fullWidth = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.body.weight(.semibold))
            .foregroundColor(Theme.brand)
            .padding(.horizontal, 20)
            .padding(.vertical, 12)
            .frame(maxWidth: fullWidth ? .infinity : nil)
            .background(Capsule().fill(Theme.brand.opacity(configuration.isPressed ? 0.12 : 0)))
            .overlay(Capsule().stroke(Theme.muted, lineWidth: 1))
            .contentShape(Capsule())
    }
}

/// Text-only button (Material's TextButton).
struct PlainTextButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.body.weight(.semibold))
            .foregroundColor(Theme.brand)
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .opacity(configuration.isPressed ? 0.6 : 1)
            .contentShape(Rectangle())
    }
}

/// A text field drawn like Material's OutlinedTextField, label above.
struct LabeledField<Field: View>: View {
    let label: String
    @ViewBuilder let field: () -> Field

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(label)
                .font(.caption)
                .foregroundColor(Theme.muted)
            field()
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                .background(RoundedRectangle(cornerRadius: 8).stroke(Theme.muted, lineWidth: 1))
        }
    }
}
