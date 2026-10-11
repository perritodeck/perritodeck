import CoreGraphics
import Foundation
import ImageIO

/// Decodes host-rendered key images (`data:image/png;base64,…` or WebP) with ImageIO.
public enum KeyImages {
    private static let prefixes = ["data:image/png;base64,", "data:image/webp;base64,"]

    /// The image bytes in a key-image data URL, or nil if it isn't one.
    public static func bytes(fromDataURL dataURL: String) -> Data? {
        guard let prefix = prefixes.first(where: { dataURL.hasPrefix($0) }) else { return nil }
        return Data(base64Encoded: String(dataURL.dropFirst(prefix.count)), options: .ignoreUnknownCharacters)
    }

    public static func decode(_ dataURL: String) -> CGImage? {
        guard let data = bytes(fromDataURL: dataURL),
              let source = CGImageSourceCreateWithData(data as CFData, nil)
        else { return nil }
        return CGImageSourceCreateImageAtIndex(source, 0, nil)
    }
}
