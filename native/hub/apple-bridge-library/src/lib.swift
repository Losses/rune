import Foundation

// Use an owned C string to avoid SwiftRs runtime export failures with Xcode 27.
@c(rune_bundle_id)
public func runeBundleId() -> UnsafeMutablePointer<CChar> {
    let bytes = (Bundle.main.bundleIdentifier ?? "").utf8CString
    let result = UnsafeMutablePointer<CChar>.allocate(capacity: bytes.count)
    bytes.withUnsafeBufferPointer { buffer in
        result.initialize(from: buffer.baseAddress!, count: buffer.count)
    }
    return result
}

@c(rune_free_bundle_id)
public func runeFreeBundleId(_ pointer: UnsafeMutablePointer<CChar>) {
    pointer.deallocate()
}
