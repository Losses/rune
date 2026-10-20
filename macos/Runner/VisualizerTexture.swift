import AppKit
import CoreVideo
import Darwin
@preconcurrency import FlutterMacOS
import QuartzCore
import os

/// Diagnostics for the texture bridge: `log stream --predicate 'subsystem == "ci.not.rune.visualizer"'`.
nonisolated private let log = Logger(subsystem: "ci.not.rune.visualizer", category: "texture")

/// Log the first few occurrences of an event, then every 300th.
nonisolated private func shouldLog(_ count: Int) -> Bool { count <= 3 || count % 300 == 0 }

/// Signatures of the C ABI exported by the Rust hub (`native/hub/src/visualizer.rs`).
private typealias FrameInfoFunction = @convention(c) (
  UnsafeMutablePointer<UInt32>?, UnsafeMutablePointer<UInt32>?
) -> UInt64
private typealias CopyFrameFunction = @convention(c) (
  UnsafeMutablePointer<UInt8>?, Int, UInt32, UInt32
) -> UInt64

/// The state read by Flutter's raster thread. Every access goes through `lock`, so the class is
/// safe to share although the compiler cannot see that.
nonisolated private final class VisualizerPixelSource: @unchecked Sendable {
  private let lock = NSLock()
  private let frameInfo: FrameInfoFunction?
  private let copyFrame: CopyFrameFunction?
  private var pool: CVPixelBufferPool?
  private var poolWidth: UInt32 = 0
  private var poolHeight: UInt32 = 0
  private var lastBuffer: CVPixelBuffer?
  private var lastFrameId: UInt64 = 0
  private var copies = 0
  private var failures = 0

  init() {
    // The symbols live in rinf.framework, which is loaded in-process; resolving them at run time
    // avoids a link-time dependency and tolerates their absence.
    if let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "rune_visualizer_frame_info") {
      frameInfo = unsafeBitCast(symbol, to: FrameInfoFunction.self)
    } else {
      frameInfo = nil
    }
    if let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "rune_visualizer_copy_frame") {
      copyFrame = unsafeBitCast(symbol, to: CopyFrameFunction.self)
    } else {
      copyFrame = nil
    }
    log.info(
      "symbols: frame_info=\(self.frameInfo != nil, privacy: .public) copy_frame=\(self.copyFrame != nil, privacy: .public)"
    )
  }

  private func fail(_ reason: StaticString, _ width: UInt32, _ height: UInt32) -> CVPixelBuffer? {
    failures += 1
    if shouldLog(failures) {
      log.error(
        "copyPixelBuffer #\(self.failures, privacy: .public) failed: \(reason, privacy: .public) (\(width, privacy: .public)x\(height, privacy: .public))"
      )
    }
    return nil
  }

  /// The id of the newest frame, or 0 if there is none (or the symbols are missing).
  func latestFrameId() -> UInt64 {
    guard let frameInfo else { return 0 }
    var width: UInt32 = 0
    var height: UInt32 = 0
    return frameInfo(&width, &height)
  }

  /// Called on the raster thread.
  func copyPixelBuffer() -> CVPixelBuffer? {
    guard let frameInfo, let copyFrame else { return nil }

    lock.lock()
    defer { lock.unlock() }

    var width: UInt32 = 0
    var height: UInt32 = 0
    let frameId = frameInfo(&width, &height)
    if frameId == 0 || width == 0 || height == 0 { return fail("no frame yet", width, height) }

    // Flutter may ask again without a new frame: hand out the previous copy.
    if frameId == lastFrameId, let lastBuffer { return lastBuffer }

    if pool == nil || poolWidth != width || poolHeight != height {
      let attributes: [CFString: Any] = [
        kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey: Int(width),
        kCVPixelBufferHeightKey: Int(height),
        kCVPixelBufferIOSurfacePropertiesKey: [CFString: Any](),
        // Flutter wraps the buffer in a Metal texture through CVMetalTextureCache.
        kCVPixelBufferMetalCompatibilityKey: true,
      ]
      var newPool: CVPixelBufferPool?
      let status = CVPixelBufferPoolCreate(nil, nil, attributes as CFDictionary, &newPool)
      log.info(
        "pool \(width, privacy: .public)x\(height, privacy: .public): status \(status, privacy: .public)"
      )
      guard status == kCVReturnSuccess else { return fail("pool creation", width, height) }
      pool = newPool
      poolWidth = width
      poolHeight = height
      lastBuffer = nil
    }
    guard let pool else { return fail("no pool", width, height) }

    var buffer: CVPixelBuffer?
    guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &buffer) == kCVReturnSuccess, let buffer
    else { return fail("buffer from pool", width, height) }

    guard CVPixelBufferLockBaseAddress(buffer, []) == kCVReturnSuccess else {
      return fail("lock base address", width, height)
    }
    defer { CVPixelBufferUnlockBaseAddress(buffer, []) }

    guard let base = CVPixelBufferGetBaseAddress(buffer) else { return fail("no base address", width, height) }
    let copiedId = copyFrame(
      base.assumingMemoryBound(to: UInt8.self),
      CVPixelBufferGetBytesPerRow(buffer),
      width,
      height)
    // 0 means the frame size changed between the two calls; try again on the next frame.
    if copiedId == 0 { return fail("copy_frame returned 0", width, height) }

    copies += 1
    if shouldLog(copies) {
      log.info(
        "copyPixelBuffer #\(self.copies, privacy: .public): frame \(copiedId, privacy: .public) \(width, privacy: .public)x\(height, privacy: .public) stride \(CVPixelBufferGetBytesPerRow(buffer), privacy: .public) iosurface \(CVPixelBufferGetIOSurface(buffer) != nil, privacy: .public)"
      )
    }
    lastBuffer = buffer
    lastFrameId = copiedId
    return buffer
  }
}

/// A Flutter texture fed by the Rust visualizer. `copyPixelBuffer` runs on the raster thread and
/// only touches the lock-protected `VisualizerPixelSource`; everything else stays on the main
/// actor.
final class VisualizerTexture: NSObject, FlutterTexture {
  private let source = VisualizerPixelSource()
  private let registry: FlutterTextureRegistry
  private weak var hostView: NSView?
  private var textureId: Int64 = 0
  // CADisplayLink exists from macOS 14; keep only a closure that stops it.
  private var stopDisplayLink: (() -> Void)?
  private var fallbackTimer: Timer?
  private var announcedFrameId: UInt64 = 0
  private var refreshes = 0
  private var announcements = 0

  init(registry: FlutterTextureRegistry, hostView: NSView?) {
    self.registry = registry
    self.hostView = hostView
    super.init()
  }

  /// Registers the texture and starts refreshing it. Returns the texture id.
  func register() -> Int64 {
    textureId = registry.register(self)
    log.info("registered texture \(self.textureId, privacy: .public)")
    startRefreshing()
    return textureId
  }

  func dispose() {
    log.info(
      "dispose texture \(self.textureId, privacy: .public) after \(self.refreshes, privacy: .public) refreshes, \(self.announcements, privacy: .public) frames announced"
    )
    stopDisplayLink?()
    stopDisplayLink = nil
    fallbackTimer?.invalidate()
    fallbackTimer = nil
    if textureId != 0 {
      registry.unregisterTexture(textureId)
      textureId = 0
    }
  }

  nonisolated func copyPixelBuffer() -> Unmanaged<CVPixelBuffer>? {
    guard let buffer = source.copyPixelBuffer() else { return nil }
    return Unmanaged.passRetained(buffer)
  }

  private func startRefreshing() {
    if #available(macOS 14.0, *), let hostView {
      let link = hostView.displayLink(target: self, selector: #selector(refresh))
      link.add(to: .main, forMode: .common)
      stopDisplayLink = { link.invalidate() }
      log.info("refresh: view display link (paused \(link.isPaused, privacy: .public))")
    } else {
      // Before macOS 14 there is no public display link on views; poll at 60 Hz instead.
      let timer = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] _ in
        // Timers on the main run loop fire on the main thread.
        MainActor.assumeIsolated {
          self?.refresh()
        }
      }
      RunLoop.main.add(timer, forMode: .common)
      fallbackTimer = timer
      log.info("refresh: 60 Hz timer (hostView \(self.hostView != nil, privacy: .public))")
    }
  }

  @objc private func refresh() {
    refreshes += 1
    let frameId = source.latestFrameId()
    if shouldLog(refreshes) {
      log.info("refresh #\(self.refreshes, privacy: .public): latest frame \(frameId, privacy: .public)")
    }
    guard frameId != 0, frameId != announcedFrameId else { return }
    announcedFrameId = frameId
    announcements += 1
    if shouldLog(announcements) {
      log.info(
        "frame available #\(self.announcements, privacy: .public): frame \(frameId, privacy: .public) texture \(self.textureId, privacy: .public)"
      )
    }
    registry.textureFrameAvailable(textureId)
  }
}

/// Owns the single visualizer texture and serves the `not.ci.rune/visualizer` method channel.
final class VisualizerChannel {
  private var texture: VisualizerTexture?

  func register(with controller: FlutterViewController) {
    // Keep the engine itself: it is the texture registry and lives as long as the app.
    let engine = controller.engine
    let channel = FlutterMethodChannel(
      name: "not.ci.rune/visualizer",
      binaryMessenger: engine.binaryMessenger)
    // The window owns this object for the app's lifetime, so the handler can hold it strongly.
    channel.setMethodCallHandler { call, result in
      // Method channel handlers run on the platform (main) thread.
      MainActor.assumeIsolated {
        switch call.method {
        case "create":
          self.texture?.dispose()
          // The view only drives the display link; without it the texture falls back to a timer.
          let texture = VisualizerTexture(registry: engine, hostView: engine.viewController?.view)
          self.texture = texture
          result(texture.register())
        case "dispose":
          self.texture?.dispose()
          self.texture = nil
          result(nil)
        default:
          result(FlutterMethodNotImplemented)
        }
      }
    }
    log.info("channel registered")
  }
}
