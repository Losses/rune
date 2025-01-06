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
@objc class AppDelegate: FlutterAppDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    let controller: FlutterViewController = window?.rootViewController as! FlutterViewController
    let channel = FlutterMethodChannel(
      name: "not.ci.rune/ios_file_selector", binaryMessenger: controller.binaryMessenger)
    channel.setMethodCallHandler({ (call: FlutterMethodCall, result: @escaping FlutterResult) in
      if call.method == "get_directory_path" {
        FileSelector.shared.getDirectoryPath(result: result)
      }
    })

    GeneratedPluginRegistrant.register(with: self)
    initAudioSession()
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }
}

class FileSelector: NSObject, UIDocumentPickerDelegate {
  static let shared = FileSelector()

  private var result: FlutterResult?
  
  private override init() {}
  
  func getDirectoryPath(result: @escaping FlutterResult) {
    self.result = result

    let documentPicker = UIDocumentPickerViewController(forOpeningContentTypes: [.folder])
    documentPicker.delegate = self
    documentPicker.allowsMultipleSelection = false
    
    if let viewController = UIApplication.shared.keyWindow?.rootViewController {
      viewController.present(documentPicker, animated: true)
    }
  }
  
  func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {  
    urls.first?.startAccessingSecurityScopedResource()
    result!(urls.first?.path)

  }
}
