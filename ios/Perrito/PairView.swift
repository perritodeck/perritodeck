import PerritoKit
import SwiftUI

/// Pair by typing the computer's address and its 6-digit code, then compare fingerprints.
/// Mirrors android/.../ui/PairScreen.kt.
struct PairView: View {
    let state: PairState
    let onSubmit: (String, String) -> Void
    let onConfirm: () -> Void
    let onCancel: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                switch state {
                case .form(let address, let error):
                    PairForm(initialAddress: address, error: error, onSubmit: onSubmit, onCancel: onCancel)
                        .id(address) // a new prefill (e.g. from a found host) resets the fields
                case .working(let message):
                    VStack(spacing: 16) {
                        ProgressView()
                            .progressViewStyle(.circular)
                            .tint(Theme.brand)
                            .scaleEffect(1.4)
                        Text(message)
                            .multilineTextAlignment(.center)
                        Button("Cancel", action: onCancel)
                            .buttonStyle(PlainTextButtonStyle())
                    }
                    .frame(maxWidth: .infinity)
                case .confirm(_, _, _, let fingerprint):
                    Text("Is this your computer?")
                        .font(.title2.weight(.semibold))
                    Spacer().frame(height: 8)
                    Text("Check that the computer shows the same fingerprint. If it doesn't, something on your network may be pretending to be it.")
                        .foregroundColor(Theme.muted)
                    Spacer().frame(height: 24)
                    Text(Fingerprint.display(fingerprint))
                        .font(.system(size: 34, design: .monospaced))
                        .foregroundColor(Theme.brand)
                        .frame(maxWidth: .infinity)
                        .accessibilityIdentifier("fingerprint")
                    Spacer().frame(height: 24)
                    HStack(spacing: 12) {
                        Button("It doesn't match", action: onCancel)
                            .buttonStyle(OutlineButtonStyle(fullWidth: true))
                        Button("It matches", action: onConfirm)
                            .buttonStyle(PrimaryButtonStyle(fullWidth: true))
                    }
                }
            }
            .frame(maxWidth: 480, alignment: .leading)
            .padding(24)
            .frame(maxWidth: .infinity)
        }
        .scrollDismissesKeyboard(.interactively)
    }
}

private struct PairForm: View {
    let error: String?
    let onSubmit: (String, String) -> Void
    let onCancel: () -> Void

    @State private var address: String
    @State private var code = ""
    @FocusState private var focused: Field?

    private enum Field {
        case address
        case code
    }

    init(initialAddress: String, error: String?, onSubmit: @escaping (String, String) -> Void, onCancel: @escaping () -> Void) {
        self.error = error
        self.onSubmit = onSubmit
        self.onCancel = onCancel
        _address = State(initialValue: initialAddress)
    }

    private func submit() {
        focused = nil
        onSubmit(address, code)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Pair with your computer")
                .font(.title.weight(.semibold))
            Spacer().frame(height: 8)
            Text("On the computer, PerritoDeck Host shows a 6-digit code. Type `pair` in its window if it isn't showing one.")
                .foregroundColor(Theme.muted)
            Spacer().frame(height: 24)

            LabeledField(label: "Computer's address") {
                TextField("192.168.1.20", text: $address)
                    .keyboardType(.URL)
                    .textContentType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.next)
                    .focused($focused, equals: .address)
                    .onSubmit { focused = .code }
                    .accessibilityIdentifier("address")
            }
            #if targetEnvironment(simulator)
            // The Simulator shares the Mac's network, so a host on the same Mac is 127.0.0.1.
            Spacer().frame(height: 8)
            Button("This Mac") { address = "127.0.0.1" }
                .buttonStyle(OutlineButtonStyle())
            #endif
            Spacer().frame(height: 8)
            LabeledField(label: "6-digit code") {
                TextField("", text: $code)
                    .keyboardType(.numberPad)
                    .textContentType(.oneTimeCode)
                    .focused($focused, equals: .code)
                    .onChange(of: code) { value in
                        let digits = String(value.filter { $0.isASCII && $0.isNumber }.prefix(6))
                        if digits != value { code = digits }
                    }
                    .accessibilityIdentifier("code")
            }

            if let error {
                Spacer().frame(height: 12)
                Text(error).foregroundColor(Theme.error)
            }

            Spacer().frame(height: 20)
            HStack(spacing: 12) {
                Button("Cancel", action: onCancel)
                    .buttonStyle(PlainTextButtonStyle())
                Spacer()
                Button("Continue", action: submit)
                    .buttonStyle(PrimaryButtonStyle())
            }
        }
    }
}
