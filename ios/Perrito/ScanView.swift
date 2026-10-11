import AVFoundation
import PerritoKit
import SwiftUI
import UIKit

/// Full-screen camera that looks for the host's pairing QR code. Calls `onLink` once with
/// the first `perritodeck://pair` link it sees. Mirrors android/.../scan/ScanScreen.kt,
/// using AVFoundation's built-in QR detection instead of ZXing.
struct ScanView: View {
    let onLink: (String) -> Void
    let onCancel: () -> Void

    @State private var status = AVCaptureDevice.authorizationStatus(for: .video)

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            switch status {
            case .authorized:
                QRScannerView(onCode: onLink)
                    .ignoresSafeArea()
                // Aiming square.
                RoundedRectangle(cornerRadius: 24)
                    .stroke(Theme.brand, lineWidth: 3)
                    .frame(width: 260, height: 260)
            case .notDetermined:
                Color.clear
            default:
                VStack(spacing: 16) {
                    Text("Perrito needs the camera only to read the pairing code. You can also pair by typing the 6-digit code instead.")
                        .multilineTextAlignment(.center)
                    // iOS asks only once; after that the choice lives in Settings.
                    Button("Allow camera") {
                        if let url = URL(string: UIApplication.openSettingsURLString) {
                            UIApplication.shared.open(url)
                        }
                    }
                    .buttonStyle(PrimaryButtonStyle())
                }
                .padding(32)
            }
            VStack {
                Spacer()
                Text("Point the camera at the QR code PerritoDeck Host shows on your computer.")
                    .foregroundColor(Theme.muted)
                    .multilineTextAlignment(.center)
                Button("Cancel", action: onCancel)
                    .buttonStyle(PlainTextButtonStyle())
            }
            .padding(24)
        }
        .onAppear {
            if status == .notDetermined {
                AVCaptureDevice.requestAccess(for: .video) { _ in
                    DispatchQueue.main.async {
                        status = AVCaptureDevice.authorizationStatus(for: .video)
                    }
                }
            }
        }
    }
}

/// The camera preview with QR detection. Reports each distinct pairing link once.
struct QRScannerView: UIViewRepresentable {
    let onCode: (String) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onCode: onCode)
    }

    func makeUIView(context: Context) -> PreviewView {
        let view = PreviewView()
        view.previewLayer.videoGravity = .resizeAspectFill
        context.coordinator.configure(view)
        return view
    }

    func updateUIView(_ uiView: PreviewView, context: Context) {
        context.coordinator.onCode = onCode
    }

    static func dismantleUIView(_ uiView: PreviewView, coordinator: Coordinator) {
        coordinator.stop()
    }

    /// A view whose layer is the camera preview, kept in the interface orientation.
    final class PreviewView: UIView {
        override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }

        var previewLayer: AVCaptureVideoPreviewLayer {
            // layerClass guarantees the type.
            layer as! AVCaptureVideoPreviewLayer
        }

        override func layoutSubviews() {
            super.layoutSubviews()
            guard let connection = previewLayer.connection,
                  connection.isVideoOrientationSupported,
                  let orientation = window?.windowScene?.interfaceOrientation
            else { return }
            switch orientation {
            case .portraitUpsideDown: connection.videoOrientation = .portraitUpsideDown
            case .landscapeLeft: connection.videoOrientation = .landscapeLeft
            case .landscapeRight: connection.videoOrientation = .landscapeRight
            default: connection.videoOrientation = .portrait
            }
        }
    }

    final class Coordinator: NSObject, AVCaptureMetadataOutputObjectsDelegate {
        var onCode: (String) -> Void
        private let session = AVCaptureSession()
        private let sessionQueue = DispatchQueue(label: "dev.perritodeck.client.camera")
        private var done = false

        init(onCode: @escaping (String) -> Void) {
            self.onCode = onCode
        }

        func configure(_ view: PreviewView) {
            view.previewLayer.session = session
            // Most devices have a back camera; use the front one if that's all there is.
            guard let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back)
                ?? AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .front),
                let input = try? AVCaptureDeviceInput(device: device)
            else { return }

            session.beginConfiguration()
            if session.canAddInput(input) { session.addInput(input) }
            let output = AVCaptureMetadataOutput()
            if session.canAddOutput(output) {
                session.addOutput(output)
                // Types can only be set once the output is attached to the session.
                if output.availableMetadataObjectTypes.contains(.qr) {
                    output.metadataObjectTypes = [.qr]
                }
                output.setMetadataObjectsDelegate(self, queue: .main)
            }
            session.commitConfiguration()

            let session = self.session
            sessionQueue.async { session.startRunning() } // blocks; keep it off the main thread
        }

        func stop() {
            let session = self.session
            sessionQueue.async { session.stopRunning() }
        }

        func metadataOutput(
            _ output: AVCaptureMetadataOutput,
            didOutput metadataObjects: [AVMetadataObject],
            from connection: AVCaptureConnection
        ) {
            guard !done else { return }
            for object in metadataObjects {
                guard let code = object as? AVMetadataMachineReadableCodeObject,
                      let text = code.stringValue,
                      text.hasPrefix("perritodeck://pair")
                else { continue }
                done = true
                stop()
                onCode(text)
                return
            }
        }
    }
}
