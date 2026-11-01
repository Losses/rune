@preconcurrency import Flutter
import UIKit
import AVFAudio

func initAudioSession() {
  let audio_session = AVAudioSession.sharedInstance();
  do {
      try audio_session.setCategory(AVAudioSession.Category.playAndRecord);
      try audio_session.setActive(true);
  } catch {
      // This is a fatal error because the audio session is required for the app to work
      fatalError("\(error)");
  }
}

/// Hands scene files (`.runescene`) the system asked Rune to open over to Dart.
///
/// Paths queue until Dart has called `takePending` once, which also covers a file that launched
/// the app before the Flutter engine existed. After that they are sent with `open` directly.
@MainActor
final class SceneFiles {
  static let shared = SceneFiles()

  private var channel: FlutterMethodChannel?
  private var pending: [String] = []
  private var dartListening = false

  private init() {}

  /// Creates the channel on the engine's messenger.
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

  /// Takes the `.runescene` files among the URLs the system handed to the app.
  func receive(_ urls: [URL]) {
    var paths: [String] = []
    for url in urls where url.isFileURL && url.pathExtension.lowercased() == "runescene" {
      if let path = SceneFiles.copyToTemporary(url) {
        paths.append(path)
      }
    }
    if paths.isEmpty {
      return
    }
    if dartListening, let channel = channel {
      channel.invokeMethod("open", arguments: paths)
    } else {
      pending.append(contentsOf: paths)
    }
  }

  /// Copies the file into a temporary folder, so that it stays readable without the security scope.
  private static func copyToTemporary(_ url: URL) -> String? {
    let scoped = url.startAccessingSecurityScopedResource()
    defer {
      if scoped {
        url.stopAccessingSecurityScopedResource()
      }
    }
    let folder = URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
      .appendingPathComponent("scene-imports", isDirectory: true)
      .appendingPathComponent(UUID().uuidString, isDirectory: true)
    let target = folder.appendingPathComponent(url.lastPathComponent)
    do {
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
      try FileManager.default.copyItem(at: url, to: target)
      return target.path
    } catch {
      NSLog("Could not copy the scene file: \(error)")
      return nil
    }
  }
}

/// Receives the files the system opens with Rune, both at launch and while it runs.
@objc(SceneDelegate)
class SceneDelegate: FlutterSceneDelegate {
  override func scene(
    _ scene: UIScene,
    willConnectTo session: UISceneSession,
    options connectionOptions: UIScene.ConnectionOptions
  ) {
    super.scene(scene, willConnectTo: session, options: connectionOptions)
    let urls = connectionOptions.urlContexts.map { $0.url }
    MainActor.assumeIsolated {
      SceneFiles.shared.receive(urls)
    }
  }

  override func scene(_ scene: UIScene, openURLContexts URLContexts: Set<UIOpenURLContext>) {
    super.scene(scene, openURLContexts: URLContexts)
    let urls = URLContexts.map { $0.url }
    MainActor.assumeIsolated {
      SceneFiles.shared.receive(urls)
    }
  }
}

@main
@objc class AppDelegate: FlutterAppDelegate, @preconcurrency FlutterImplicitEngineDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    initAudioSession()
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  // Flutter invokes this delegate on the main thread, but its protocol is not actor-annotated.
  @MainActor
  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)

    let channel = FlutterMethodChannel(
      name: "not.ci.rune/ios_file_selector",
      binaryMessenger: engineBridge.applicationRegistrar.messenger())
    channel.setMethodCallHandler({ (call: FlutterMethodCall, result: @escaping FlutterResult) in
      if call.method == "get_directory_path" {
        FileSelector.shared.getDirectoryPath(result: result)
      }
    })

    SceneFiles.shared.attach(messenger: engineBridge.applicationRegistrar.messenger())

    // The Rust hub registers the visualizer's texture with the engine itself. Its symbols live in
    // rinf.framework, which is loaded in-process; resolving at run time avoids a link-time
    // dependency.
    if let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "rune_flutter_attach"),
      let textures = engineBridge.pluginRegistry.registrar(forPlugin: "RuneFlutterTexture")?
        .textures()
    {
      typealias Attach = @convention(c) (UnsafeMutableRawPointer) -> Bool
      let attach = unsafeBitCast(symbol, to: Attach.self)
      _ = attach(Unmanaged.passUnretained(textures as AnyObject).toOpaque())
    }
  }
}

@MainActor
class FileSelector: NSObject, UIDocumentPickerDelegate {
  static let shared = FileSelector()

  private var result: FlutterResult?
  
  private override init() {}
  
  func getDirectoryPath(result: @escaping FlutterResult) {
    let activeWindow = UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }
      .filter { $0.activationState == .foregroundActive }
      .flatMap { $0.windows }
      .first { $0.isKeyWindow }
    guard let viewController = activeWindow?.rootViewController else {
      result(FlutterError(code: "no_active_window", message: "No active window for the directory picker", details: nil))
      return
    }

    self.result = result

    let documentPicker = UIDocumentPickerViewController(forOpeningContentTypes: [.folder])
    documentPicker.delegate = self
    documentPicker.allowsMultipleSelection = false
    
    viewController.present(documentPicker, animated: true)
  }
  
  func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {  
    urls.first?.startAccessingSecurityScopedResource()
    result!(urls.first?.path)

  }
}
