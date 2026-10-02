import CoreGraphics
import Darwin
import Foundation
import ImageIO
import PDFKit
import Vision

// Status codes shared with src/parsers/ocr/vision.rs.
private let statusOk: Int32 = 0
private let statusTimeout: Int32 = 1
private let statusInvalidDocument: Int32 = 2
private let statusFailed: Int32 = 3

private let renderDotsPerInch: CGFloat = 300
private let recognitionRevision = VNRecognizeTextRequestRevision3

private struct OcrFailure: Error {
    let status: Int32
    let message: String
}

private enum OcrPages {
    case pdf(PDFDocument)
    case image(CGImageSource)
}

private final class OcrSource {
    let pages: OcrPages
    let pageCount: Int
    let maximumPagePixels: Int

    init(pages: OcrPages, pageCount: Int, maximumPagePixels: Int) {
        self.pages = pages
        self.pageCount = pageCount
        self.maximumPagePixels = maximumPagePixels
    }
}

private final class RecognitionResult: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Result<String, OcrFailure>?

    func store(_ result: Result<String, OcrFailure>) {
        lock.lock()
        value = result
        lock.unlock()
    }

    func load() -> Result<String, OcrFailure>? {
        lock.lock()
        defer { lock.unlock() }
        return value
    }
}

private func report(_ message: String, into slot: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) {
    slot?.pointee = strdup(message)
}

/// The scale that renders `width` x `height` points at 300 DPI, reduced so the bitmap stays within
/// `maximumPixels`. An oversized page is rendered at lower resolution rather than refused.
private func boundedScale(width: CGFloat, height: CGFloat, maximumPixels: Int) -> CGFloat {
    let preferred = renderDotsPerInch / 72
    let area = width * height * preferred * preferred
    guard area > CGFloat(maximumPixels) else { return preferred }
    return preferred * (CGFloat(maximumPixels) / area).squareRoot()
}

private func renderPdfPage(_ page: PDFPage, maximumPixels: Int) throws -> CGImage {
    let box = PDFDisplayBox.cropBox
    let bounds = page.bounds(for: box)
    let quarterTurns = ((page.rotation / 90) % 4 + 4) % 4
    let width = quarterTurns % 2 == 0 ? bounds.width : bounds.height
    let height = quarterTurns % 2 == 0 ? bounds.height : bounds.width
    guard width.isFinite, height.isFinite, width > 0, height > 0 else {
        throw OcrFailure(status: statusInvalidDocument, message: "OCR_PAGE_GEOMETRY_INVALID: page has no drawable area")
    }
    let scale = boundedScale(width: width, height: height, maximumPixels: maximumPixels)
    let pixelWidth = max(1, Int((width * scale).rounded(.down)))
    let pixelHeight = max(1, Int((height * scale).rounded(.down)))
    guard
        let context = CGContext(
            data: nil,
            width: pixelWidth,
            height: pixelHeight,
            bitsPerComponent: 8,
            bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceGray(),
            bitmapInfo: CGImageAlphaInfo.none.rawValue
        )
    else {
        throw OcrFailure(status: statusFailed, message: "OCR_RASTER_ALLOCATION_FAILED: \(pixelWidth)x\(pixelHeight) page bitmap")
    }
    // A bitmap without alpha starts black; transparent page backgrounds must read as paper.
    context.setFillColor(gray: 1, alpha: 1)
    context.fill(CGRect(x: 0, y: 0, width: pixelWidth, height: pixelHeight))
    context.scaleBy(x: scale, y: scale)
    // draw(with:to:) applies the crop-box origin and /Rotate itself; an extra transform(_:for:)
    // would rotate the page twice.
    page.draw(with: box, to: context)
    guard let image = context.makeImage() else {
        throw OcrFailure(status: statusFailed, message: "OCR_RASTER_FAILED: page bitmap could not be read back")
    }
    return image
}

private func decodeImageFrame(_ source: CGImageSource, index: Int, maximumPixels: Int) throws -> CGImage {
    guard
        let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [CFString: Any],
        let width = (properties[kCGImagePropertyPixelWidth] as? NSNumber)?.doubleValue,
        let height = (properties[kCGImagePropertyPixelHeight] as? NSNumber)?.doubleValue,
        width >= 1, height >= 1
    else {
        throw OcrFailure(status: statusInvalidDocument, message: "OCR_IMAGE_DIMENSIONS_MISSING: frame \(index + 1) has no pixel dimensions")
    }
    // Dimensions come from the header, so a decompression bomb is downsampled before decode
    // rather than materialized at full size.
    let scale = min(1, (Double(maximumPixels) / (width * height)).squareRoot())
    let longestSide = max(1, Int((max(width, height) * scale).rounded(.down)))
    let options: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceThumbnailMaxPixelSize: longestSide,
        kCGImageSourceShouldCacheImmediately: true,
    ]
    guard let image = CGImageSourceCreateThumbnailAtIndex(source, index, options as CFDictionary) else {
        throw OcrFailure(status: statusInvalidDocument, message: "OCR_IMAGE_DECODE_FAILED: frame \(index + 1) could not be decoded")
    }
    return image
}

private func pageImage(_ source: OcrSource, index: Int) throws -> CGImage {
    switch source.pages {
    case .pdf(let document):
        guard let page = document.page(at: index) else {
            throw OcrFailure(status: statusInvalidDocument, message: "OCR_PAGE_UNAVAILABLE: page \(index + 1) could not be loaded")
        }
        return try renderPdfPage(page, maximumPixels: source.maximumPagePixels)
    case .image(let imageSource):
        return try decodeImageFrame(imageSource, index: index, maximumPixels: source.maximumPagePixels)
    }
}

private func recognize(_ image: CGImage, request: VNRecognizeTextRequest) throws -> String {
    let handler = VNImageRequestHandler(cgImage: image, options: [:])
    do {
        try handler.perform([request])
    } catch {
        throw OcrFailure(status: statusFailed, message: "OCR_ENGINE_FAILED: \(error.localizedDescription)")
    }
    let lines = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
    return lines.joined(separator: "\n")
}

private func makeRequest() -> VNRecognizeTextRequest {
    let request = VNRecognizeTextRequest()
    request.revision = recognitionRevision
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = true
    request.automaticallyDetectsLanguage = true
    return request
}

@_cdecl("onrecord_vision_open")
public func onrecordVisionOpen(
    _ path: UnsafePointer<CChar>,
    _ isPdf: Bool,
    _ maximumPagePixels: Int,
    _ status: UnsafeMutablePointer<Int32>,
    _ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> UnsafeMutableRawPointer? {
    let url = URL(fileURLWithPath: String(cString: path))
    let source: OcrSource
    if isPdf {
        guard let document = PDFDocument(url: url) else {
            status.pointee = statusInvalidDocument
            report("OCR_PDF_OPEN_FAILED: PDFKit could not open the document", into: error)
            return nil
        }
        if document.isLocked {
            status.pointee = statusInvalidDocument
            report("OCR_PDF_ENCRYPTED: the document requires a password", into: error)
            return nil
        }
        source = OcrSource(pages: .pdf(document), pageCount: document.pageCount, maximumPagePixels: maximumPagePixels)
    } else {
        guard
            let imageSource = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
            CGImageSourceGetStatus(imageSource) == .statusComplete
        else {
            status.pointee = statusInvalidDocument
            report("OCR_IMAGE_OPEN_FAILED: the image could not be read", into: error)
            return nil
        }
        source = OcrSource(pages: .image(imageSource), pageCount: CGImageSourceGetCount(imageSource), maximumPagePixels: maximumPagePixels)
    }
    status.pointee = statusOk
    return Unmanaged.passRetained(source).toOpaque()
}

@_cdecl("onrecord_vision_page_count")
public func onrecordVisionPageCount(_ handle: UnsafeMutableRawPointer) -> Int {
    Unmanaged<OcrSource>.fromOpaque(handle).takeUnretainedValue().pageCount
}

/// Renders and recognizes one page. The caller's thread waits at most `timeoutSeconds`; on expiry
/// the request is cancelled and the in-flight work keeps its own references until it unwinds, so
/// the handle may be closed immediately.
@_cdecl("onrecord_vision_recognize_page")
public func onrecordVisionRecognizePage(
    _ handle: UnsafeMutableRawPointer,
    _ index: Int,
    _ timeoutSeconds: Double,
    _ status: UnsafeMutablePointer<Int32>,
    _ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> UnsafeMutablePointer<CChar>? {
    let source = Unmanaged<OcrSource>.fromOpaque(handle).takeUnretainedValue()
    guard index >= 0, index < source.pageCount else {
        status.pointee = statusInvalidDocument
        report("OCR_PAGE_UNAVAILABLE: page \(index + 1) is out of range", into: error)
        return nil
    }
    let request = makeRequest()
    let result = RecognitionResult()
    let finished = DispatchSemaphore(value: 0)
    DispatchQueue.global(qos: .userInitiated).async {
        autoreleasepool {
            do {
                let image = try pageImage(source, index: index)
                result.store(.success(try recognize(image, request: request)))
            } catch let failure as OcrFailure {
                result.store(.failure(failure))
            } catch {
                result.store(.failure(OcrFailure(status: statusFailed, message: "OCR_ENGINE_FAILED: \(error.localizedDescription)")))
            }
        }
        finished.signal()
    }
    guard timeoutSeconds > 0, finished.wait(timeout: .now() + timeoutSeconds) == .success else {
        request.cancel()
        status.pointee = statusTimeout
        report("OCR_ENGINE_TIMEOUT: processing deadline expired on page \(index + 1)", into: error)
        return nil
    }
    switch result.load() {
    case .success(let text):
        status.pointee = statusOk
        return strdup(text)
    case .failure(let failure):
        status.pointee = failure.status
        report(failure.message, into: error)
        return nil
    case nil:
        status.pointee = statusFailed
        report("OCR_ENGINE_FAILED: recognition finished without a result", into: error)
        return nil
    }
}

@_cdecl("onrecord_vision_close")
public func onrecordVisionClose(_ handle: UnsafeMutableRawPointer) {
    Unmanaged<OcrSource>.fromOpaque(handle).release()
}

/// JSON: {"revision": Int, "osVersion": String, "languages": [String], "languageError": String?}
@_cdecl("onrecord_vision_describe")
public func onrecordVisionDescribe() -> UnsafeMutablePointer<CChar>? {
    let request = makeRequest()
    var description: [String: Any] = [
        "revision": recognitionRevision,
        "osVersion": ProcessInfo.processInfo.operatingSystemVersionString,
    ]
    do {
        description["languages"] = try request.supportedRecognitionLanguages()
    } catch {
        description["languages"] = [String]()
        description["languageError"] = error.localizedDescription
    }
    guard
        let data = try? JSONSerialization.data(withJSONObject: description),
        let json = String(data: data, encoding: .utf8)
    else {
        return nil
    }
    return strdup(json)
}

@_cdecl("onrecord_vision_free_string")
public func onrecordVisionFreeString(_ value: UnsafeMutablePointer<CChar>?) {
    free(value)
}
