// Decode QR codes in a local raster image using macOS Vision.
// Usage: swift scripts/decode-qr.swift path/to/image.png
import Foundation
import ImageIO
import Vision

func fail(_ message: String) -> Never {
    fputs("decode-qr: \(message)\n", stderr)
    exit(1)
}

guard CommandLine.arguments.count == 2 else {
    fail("usage: swift scripts/decode-qr.swift IMAGE_PATH")
}

let imageURL = URL(fileURLWithPath: CommandLine.arguments[1])
guard let source = CGImageSourceCreateWithURL(imageURL as CFURL, nil),
      let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
    fail("cannot read image")
}

let request = VNDetectBarcodesRequest()
request.symbologies = [.qr]

do {
    try VNImageRequestHandler(cgImage: image).perform([request])
} catch {
    fail("barcode detection failed: \(error.localizedDescription)")
}

let payloads = (request.results ?? []).compactMap(\.payloadStringValue)
guard !payloads.isEmpty else {
    fail("no QR code found")
}

do {
    let json = try JSONSerialization.data(withJSONObject: payloads, options: [.fragmentsAllowed])
    guard let output = String(data: json, encoding: .utf8) else {
        fail("cannot encode JSON output")
    }
    print(output)
} catch {
    fail("cannot encode JSON output: \(error.localizedDescription)")
}
