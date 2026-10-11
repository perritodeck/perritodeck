import PerritoKit
import SwiftUI

/// Paired computers, computers found on the network, and the ways to pair.
/// Mirrors android/.../ui/HomeScreen.kt.
struct HomeView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var discovery: HostBrowser

    private let canScan = DeviceInfo.hasCamera

    var body: some View {
        let pairedIds = Set(model.paired.map(\.hostId))
        let unpairedFound = discovery.hosts.filter { $0.hostId == nil || !pairedIds.contains($0.hostId!) }
        let onlineIds = Set(discovery.hosts.compactMap(\.hostId))

        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Text("Perrito")
                    .font(.largeTitle.weight(.bold))
                Spacer().frame(height: 4)
                Text("Your \(DeviceInfo.model)'s button deck. Pair it with a computer running PerritoDeck Host.")
                    .foregroundColor(Theme.muted)

                if let notice = model.notice {
                    Spacer().frame(height: 16)
                    Text(notice).foregroundColor(Theme.error)
                }

                if !model.paired.isEmpty {
                    SectionTitle("Your computers")
                    ForEach(model.paired) { host in
                        HostRow(
                            title: host.name,
                            detail: onlineIds.contains(host.hostId)
                                ? "On this network"
                                : HostPort(host: host.address, port: host.port).display
                        ) {
                            Button("Forget") { model.forget(host) }
                                .buttonStyle(PlainTextButtonStyle())
                            Button("Connect") { model.connect(to: host) }
                                .buttonStyle(PrimaryButtonStyle())
                        }
                    }
                }

                if !unpairedFound.isEmpty {
                    SectionTitle("Found on this network")
                    ForEach(unpairedFound) { host in
                        let address = HostPort(host: host.address, port: host.port).display
                        HostRow(title: host.name, detail: address) {
                            Button("Pair") { model.openPairing(prefillAddress: address) }
                                .buttonStyle(PrimaryButtonStyle())
                        }
                    }
                }

                Spacer().frame(height: 24)
                if canScan {
                    Button("Scan QR code") { model.openScan() }
                        .buttonStyle(PrimaryButtonStyle(fullWidth: true))
                    Spacer().frame(height: 8)
                }
                Button("Pair with a code") { model.openPairing() }
                    .buttonStyle(OutlineButtonStyle(fullWidth: true))

                if model.paired.isEmpty && unpairedFound.isEmpty {
                    Spacer().frame(height: 12)
                    Text("Start PerritoDeck Host on your computer. It shows a QR code you can scan with this \(DeviceInfo.model)'s camera, or a 6-digit code to enter here.")
                        .foregroundColor(Theme.muted)
                }
            }
            .frame(maxWidth: 560, alignment: .leading)
            .padding(24)
            .frame(maxWidth: .infinity)
        }
        .onAppear { discovery.start() }
        .onDisappear { discovery.stop() }
    }
}

private struct SectionTitle: View {
    let title: String

    init(_ title: String) {
        self.title = title
    }

    var body: some View {
        Text(title)
            .font(.headline)
            .padding(.top, 28)
            .padding(.bottom, 8)
    }
}

private struct HostRow<Actions: View>: View {
    let title: String
    let detail: String
    @ViewBuilder let actions: () -> Actions

    var body: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                Text(detail)
                    .font(.footnote)
                    .foregroundColor(Theme.muted)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            actions()
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .background(RoundedRectangle(cornerRadius: 12).fill(Theme.surface))
        .padding(.vertical, 4)
    }
}
