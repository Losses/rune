import Flutter
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
