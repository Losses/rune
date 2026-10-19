import AppKit
import CoreVideo
import Darwin
@preconcurrency import FlutterMacOS
import QuartzCore

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
    if frameId == 0 || width == 0 || height == 0 { return nil }

    // Flutter may ask again without a new frame: hand out the previous copy.
    if frameId == lastFrameId, let lastBuffer { return lastBuffer }

    if pool == nil || poolWidth != width || poolHeight != height {
      let attributes: [CFString: Any] = [
        kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey: Int(width),
        kCVPixelBufferHeightKey: Int(height),
        kCVPixelBufferIOSurfacePropertiesKey: [CFString: Any](),
      ]
      var newPool: CVPixelBufferPool?
      guard
        CVPixelBufferPoolCreate(nil, nil, attributes as CFDictionary, &newPool) == kCVReturnSuccess
      else { return nil }
      pool = newPool
      poolWidth = width
      poolHeight = height
      lastBuffer = nil
    }
    guard let pool else { return nil }

    var buffer: CVPixelBuffer?
    guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &buffer) == kCVReturnSuccess, let buffer
    else { return nil }

    guard CVPixelBufferLockBaseAddress(buffer, []) == kCVReturnSuccess else { return nil }
    defer { CVPixelBufferUnlockBaseAddress(buffer, []) }

    guard let base = CVPixelBufferGetBaseAddress(buffer) else { return nil }
    let copiedId = copyFrame(
      base.assumingMemoryBound(to: UInt8.self),
      CVPixelBufferGetBytesPerRow(buffer),
      width,
      height)
    // 0 means the frame size changed between the two calls; try again on the next frame.
    if copiedId == 0 { return nil }

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

  init(registry: FlutterTextureRegistry, hostView: NSView?) {
    self.registry = registry
    self.hostView = hostView
    super.init()
  }

  /// Registers the texture and starts refreshing it. Returns the texture id.
  func register() -> Int64 {
    textureId = registry.register(self)
    startRefreshing()
    return textureId
  }

  func dispose() {
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
    }
  }

  @objc private func refresh() {
    let frameId = source.latestFrameId()
    guard frameId != 0, frameId != announcedFrameId else { return }
    announcedFrameId = frameId
    registry.textureFrameAvailable(textureId)
  }
}

/// Owns the single visualizer texture and serves the `not.ci.rune/visualizer` method channel.
final class VisualizerChannel {
  private var texture: VisualizerTexture?

  func register(with controller: FlutterViewController) {
    let channel = FlutterMethodChannel(
      name: "not.ci.rune/visualizer",
      binaryMessenger: controller.engine.binaryMessenger)
    channel.setMethodCallHandler { [weak self, weak controller] call, result in
      // Method channel handlers run on the platform (main) thread.
      MainActor.assumeIsolated {
        switch call.method {
        case "create":
          guard let self, let controller else {
            result(FlutterError(code: "unavailable", message: "No Flutter view", details: nil))
            return
          }
          self.texture?.dispose()
          let texture = VisualizerTexture(
            registry: controller.engine, hostView: controller.view)
          self.texture = texture
          result(texture.register())
        case "dispose":
          self?.texture?.dispose()
          self?.texture = nil
          result(nil)
        default:
          result(FlutterMethodNotImplemented)
        }
      }
    }
  }
}
