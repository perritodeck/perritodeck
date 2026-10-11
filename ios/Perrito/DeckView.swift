import PerritoKit
import SwiftUI
import UIKit

/// The deck: the host's grid of keys and sliders, fitted to the screen.
/// Mirrors android/.../ui/DeckScreen.kt.
struct DeckView: View {
    @ObservedObject var connection: DeckConnection
    let onDisconnect: () -> Void

    /// ✓ (true) or ⚠ (false) showing on a key, until it fades.
    @State private var badges: [Int: KeyFlash] = [:]

    var body: some View {
        ZStack {
            Theme.background.ignoresSafeArea()

            if let layout = connection.layout {
                KeyGrid(
                    layout: layout,
                    images: connection.images,
                    badges: badges,
                    onKeyDown: { connection.keyDown($0) },
                    onKeyUp: { connection.keyUp($0) },
                    onSlider: { index, value, final in connection.sliderChange(index, value: value, final: final) }
                )
            }

            if case .connected(let welcome) = connection.state {
                StatusBadge(hostName: welcome.hostName, rttMs: connection.rttMs, onDisconnect: onDisconnect)
            }

            if !connection.state.isConnected || connection.layout == nil {
                ConnectingOverlay(state: connection.state, onDisconnect: onDisconnect)
            }
        }
        .statusBarHidden(true)
        .persistentSystemOverlays(.hidden)
        // Keep the screen awake while the deck shows.
        .onAppear { UIApplication.shared.isIdleTimerDisabled = true }
        .onDisappear { UIApplication.shared.isIdleTimerDisabled = false }
        .onChange(of: connection.flash) { flash in
            guard let flash else { return }
            // A problem gets a stronger buzz than a key press.
            if !flash.ok { Haptics.warning() }
            badges[flash.keyIndex] = flash
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: 1_200_000_000)
                if badges[flash.keyIndex] == flash { badges[flash.keyIndex] = nil }
            }
        }
    }
}

/// Host name and the last key's round trip, top right, with a menu to disconnect.
private struct StatusBadge: View {
    let hostName: String
    let rttMs: Double?
    let onDisconnect: () -> Void

    var body: some View {
        VStack {
            HStack(spacing: 6) {
                Spacer()
                if let rttMs {
                    Text("\(hostName) · \(rttMs, specifier: "%.1f") ms")
                        .font(.caption2)
                        .foregroundColor(Theme.muted)
                }
                // Two taps to leave, so a stray touch on the edge doesn't end the session.
                Menu {
                    Button("Disconnect", role: .destructive, action: onDisconnect)
                } label: {
                    Image(systemName: "ellipsis.circle")
                        .font(.caption)
                        .foregroundColor(Theme.muted)
                        .padding(4)
                }
                .accessibilityLabel("Deck menu")
            }
            .padding(.horizontal, 8)
            Spacer()
        }
    }
}

private struct ConnectingOverlay: View {
    let state: DeckConnection.State
    let onDisconnect: () -> Void

    private var message: String {
        switch state {
        case .connecting(let url, _, let lastError):
            if let lastError { return "Reconnecting… (\(lastError))" }
            return "Connecting to \(url)…"
        default:
            return ""
        }
    }

    var body: some View {
        ZStack {
            Color.black.opacity(0.6).ignoresSafeArea()
            VStack(spacing: 16) {
                ProgressView()
                    .progressViewStyle(.circular)
                    .tint(Theme.brand)
                    .scaleEffect(1.4)
                Text(message)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 24)
                Button("Disconnect", action: onDisconnect)
                    .buttonStyle(PlainTextButtonStyle())
            }
        }
    }
}

private struct KeyGrid: View {
    /// Qualified: SwiftUI has its own `Layout` protocol.
    let layout: PerritoKit.Layout
    let images: [Int: CGImage]
    let badges: [Int: KeyFlash]
    let onKeyDown: (Int) -> Void
    let onKeyUp: (Int) -> Void
    let onSlider: (Int, Int, Bool) -> Void

    var body: some View {
        GeometryReader { geo in
            let m = GridMetrics.fit(
                width: Double(geo.size.width),
                height: Double(geo.size.height),
                cols: layout.cols,
                rows: layout.rows
            )
            let byIndex = Dictionary(layout.keys.map { ($0.index, $0) }, uniquingKeysWith: { _, last in last })
            let cell = CGFloat(m.cell)
            VStack(spacing: CGFloat(m.gap)) {
                ForEach(0..<m.rows, id: \.self) { row in
                    HStack(spacing: CGFloat(m.gap)) {
                        ForEach(0..<m.cols, id: \.self) { col in
                            let index = m.keyIndex(row: row, col: col)
                            let spec = byIndex[index]
                            if let spec, spec.isSlider {
                                SliderCell(
                                    index: index,
                                    spec: spec,
                                    alert: badges[index].map { !$0.ok } ?? false,
                                    size: cell,
                                    onSlider: onSlider
                                )
                            } else {
                                KeyCell(
                                    index: index,
                                    spec: spec,
                                    image: images[index],
                                    badge: badges[index]?.ok,
                                    size: cell,
                                    onDown: onKeyDown,
                                    onUp: onKeyUp
                                )
                            }
                        }
                    }
                }
            }
            .frame(width: geo.size.width, height: geo.size.height)
        }
    }
}

/// One key. Each key tracks its own touch, so several keys can be held at once.
private struct KeyCell: View {
    let index: Int
    let spec: KeySpec?
    let image: CGImage?
    let badge: Bool?
    let size: CGFloat
    let onDown: (Int) -> Void
    let onUp: (Int) -> Void

    /// True while a finger is down. Resets on lift and on cancellation, so keyUp always follows keyDown.
    @GestureState private var pressed = false

    private var title: String { spec?.title ?? "" }

    var body: some View {
        let background = spec.flatMap { Color(hex: $0.bgColor) } ?? Theme.surface
        let foreground = spec.flatMap { Color(hex: $0.textColor) } ?? Theme.onSurface
        let shape = RoundedRectangle(cornerRadius: size * 0.14, style: .continuous)
        let fontSize = size * 0.16

        ZStack {
            shape.fill(pressed ? background.opacity(0.7) : background)
            if let image {
                // Host-rendered key: icon and title are already drawn into the image.
                Image(decorative: image, scale: 1)
                    .resizable()
                    .interpolation(.medium)
                    .scaledToFill()
                    .frame(width: size, height: size)
            } else {
                Text(title)
                    .font(.system(size: fontSize, weight: .semibold))
                    .foregroundColor(foreground)
                    .multilineTextAlignment(.center)
                    .lineLimit(3)
                    .truncationMode(.tail)
                    .padding(size * 0.06)
            }
            if let badge {
                Color.black.opacity(0.55)
                Text(badge ? "✓" : "⚠")
                    .font(.system(size: size * 0.42, weight: .bold))
                    .foregroundColor(badge ? Theme.ok : Theme.warning)
            }
        }
        .frame(width: size, height: size)
        .clipShape(shape)
        .scaleEffect(pressed ? 0.94 : 1)
        .contentShape(shape)
        .gesture(
            DragGesture(minimumDistance: 0)
                .updating($pressed) { _, state, _ in state = true }
        )
        .onChange(of: pressed) { down in
            if down {
                Haptics.press()
                onDown(index)
            } else {
                onUp(index)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Key \(index + 1): \(title)")
        .accessibilityAddTraits(.isButton)
        .accessibilityAction {
            onDown(index)
            onUp(index)
        }
    }
}

/// A slider filling one cell: drag up and down. The level shows as a fill from the bottom;
/// while the finger is down the slider follows it, otherwise it follows the host (which
/// reports changes made on the computer).
private struct SliderCell: View {
    let index: Int
    let spec: KeySpec
    let alert: Bool
    let size: CGFloat
    let onSlider: (Int, Int, Bool) -> Void

    /// The finger's value while dragging; nil after lift or cancellation.
    @GestureState private var dragValue: Int? = nil
    /// Last value reported for this drag (for ticks and the final send).
    @State private var lastSent: Int? = nil

    var body: some View {
        let value = clampPercent(dragValue ?? spec.value ?? 0)
        let background = Color(hex: spec.bgColor) ?? Theme.surface
        let foreground = Color(hex: spec.textColor) ?? Theme.onSurface
        let shape = RoundedRectangle(cornerRadius: size * 0.14, style: .continuous)
        let fontSize = size * 0.14

        ZStack(alignment: .bottom) {
            background.opacity(0.35)
            background.frame(height: size * CGFloat(value) / 100)
            VStack(spacing: 0) {
                Text(spec.title)
                    .font(.system(size: fontSize, weight: .semibold))
                    .multilineTextAlignment(.center)
                    .lineLimit(2)
                    .truncationMode(.tail)
                Text(alert ? "⚠" : "\(value)")
                    .font(.system(size: fontSize * 1.4, weight: .bold))
            }
            .foregroundColor(foreground)
            .padding(.top, size * 0.08)
            .padding(.horizontal, size * 0.04)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
        .frame(width: size, height: size)
        .clipShape(shape)
        .contentShape(shape)
        .gesture(
            DragGesture(minimumDistance: 0)
                .updating($dragValue) { g, state, _ in
                    state = SliderMath.value(atY: Double(g.location.y), height: Double(size))
                }
                .onChanged { g in
                    let v = SliderMath.value(atY: Double(g.location.y), height: Double(size))
                    if let last = lastSent {
                        guard v != last else { return }
                        if SliderMath.crossesTick(from: last, to: v) { Haptics.tick() }
                    } else {
                        Haptics.press()
                    }
                    lastSent = v
                    onSlider(index, v, false)
                }
        )
        // dragValue returns to nil on lift and on cancellation: send the final value once.
        .onChange(of: dragValue == nil) { ended in
            if ended, let v = lastSent {
                lastSent = nil
                onSlider(index, v, true)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(spec.title) slider")
        .accessibilityValue("\(value) percent")
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: onSlider(index, clampPercent(value + 10), true)
            case .decrement: onSlider(index, clampPercent(value - 10), true)
            @unknown default: break
            }
        }
    }
}
