import Cocoa
@preconcurrency import FlutterMacOS

/// Hands scene files (`.runescene`) the system asked Rune to open over to Dart. It is one of the
/// app delegate's lifecycle delegates, so other URLs still reach the plugins.
///
/// Paths queue until Dart has called `takePending` once, which also covers a file that launched
/// the app before the Flutter engine existed. After that they are sent with `open` directly.
@MainActor
final class SceneFiles: NSObject, FlutterAppLifecycleDelegate {
  static let shared = SceneFiles()

  private var channel: FlutterMethodChannel?
  private var pending: [String] = []
  private var dartListening = false

  private override init() {}

  /// Creates the channel on the engine's messenger. Called once the Flutter view controller exists.
  func attach(messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(name: "ci.not.rune/scene_files", binaryMessenger: messenger)
    channel.setMethodCallHandler { [weak self] (call, result) in
      guard call.method == "takePending" else {
        result(FlutterMethodNotImplemented)
        return
      }
      // Flutter delivers channel calls on the main thread.
      MainActor.assumeIsolated {
        let paths = self?.pending ?? []
        self?.pending = []
        self?.dartListening = true
        result(paths)
      }
    }
    self.channel = channel
  }

  /// Takes the `.runescene` files among the URLs the system opens with Rune; false (leaving them
  /// to the plugins) when there are none.
  func handleOpen(_ urls: [URL]) -> Bool {
    let paths = urls
      .filter { $0.isFileURL && $0.pathExtension.lowercased() == "runescene" }
      .map { $0.path }
    if paths.isEmpty {
      return false
    }
    if dartListening, let channel = channel {
      channel.invokeMethod("open", arguments: paths)
    } else {
      pending.append(contentsOf: paths)
    }
    return true
  }
}

@main
class AppDelegate: FlutterAppDelegate {
  nonisolated override init() {
    super.init()
  }

  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    return false
  }

  override func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
    NSApp.windows.first?.makeKeyAndOrderFront(nil)
    return true
  }
}
