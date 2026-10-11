import AVFoundation
import PerritoKit
import UIKit

/// Haptic feedback. iPhones have a Taptic Engine; iPads have none, and these do nothing there.
@MainActor
enum Haptics {
    private static let impact = UIImpactFeedbackGenerator(style: .light)
    private static let selection = UISelectionFeedbackGenerator()
    private static let notification = UINotificationFeedbackGenerator()

    /// A key or slider was touched.
    static func press() {
        impact.impactOccurred()
        impact.prepare()
    }

    /// A slider crossed a tenth, like a detent.
    static func tick() {
        selection.selectionChanged()
        selection.prepare()
    }

    /// The host reported a problem with a key's action (a longer buzz than a press).
    static func warning() {
        notification.notificationOccurred(.warning)
    }
}

/// What this device tells the host in `hello`.
@MainActor
enum DeviceInfo {
    /// iOS 16+ reports a generic name ("iPad", "iPhone") unless the app has a special entitlement.
    static var name: String { UIDevice.current.name }

    /// "iPad" or "iPhone", for copy that mentions the device.
    static var model: String { UIDevice.current.model }

    static var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.0.0"
    }

    nonisolated static var hasCamera: Bool {
        AVCaptureDevice.default(for: .video) != nil
    }

    static func hello(deviceId: String, secret: String) -> Hello {
        Hello(
            deviceId: deviceId,
            deviceName: name,
            appVersion: appVersion,
            platform: .ios,
            osVersion: UIDevice.current.systemVersion,
            screen: screen(),
            capabilities: Hello.clientCapabilities,
            deviceSecret: secret
        )
    }

    /// Screen size in pixels in the current orientation (as Android reports it), and a
    /// density in Android's terms: 160 dpi per point scale, so 2× is 320 and 3× is 480.
    static func screen() -> Screen {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let scene = scenes.first(where: { $0.activationState == .foregroundActive }) ?? scenes.first
        guard let screen = scene?.screen else {
            return Screen(widthPx: 0, heightPx: 0, densityDpi: 160)
        }
        let native = screen.nativeBounds.size // always portrait
        let landscape = screen.bounds.width > screen.bounds.height
        let short = Int(min(native.width, native.height))
        let long = Int(max(native.width, native.height))
        return Screen(
            widthPx: landscape ? long : short,
            heightPx: landscape ? short : long,
            densityDpi: Int((160 * screen.nativeScale).rounded())
        )
    }
}
