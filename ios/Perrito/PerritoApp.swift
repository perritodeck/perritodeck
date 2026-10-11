import PerritoKit
import SwiftUI

@main
struct PerritoApp: App {
    @StateObject private var model = AppModel()

    var body: some Scene {
        WindowGroup {
            RootView(model: model)
                .preferredColorScheme(.dark)
                .tint(Theme.brand)
                // A pairing QR code scanned with the Camera app opens perritodeck://pair?…
                .onOpenURL { url in
                    if url.scheme == "perritodeck" { model.handleLink(url.absoluteString) }
                }
        }
    }
}

struct RootView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        ZStack {
            Theme.background.ignoresSafeArea()
            switch model.route {
            case .home:
                HomeView(model: model, discovery: model.discovery)
            case .pair:
                PairView(
                    state: model.pairState,
                    onSubmit: { address, code in model.submitCode(address: address, code: code) },
                    onConfirm: { model.confirmPairing() },
                    onCancel: { model.closePairing() }
                )
            case .scan:
                ScanView(onLink: { model.handleLink($0) }, onCancel: { model.closePairing() })
            case .deck:
                DeckView(connection: model.connection, onDisconnect: { model.disconnect() })
            }
        }
        .foregroundColor(Theme.onSurface)
    }
}
