import AppKit
import Cocoa
import FlutterMacOS
import bitsdojo_window_macos

class MainFlutterWindow: BitsdojoWindow {
  override func bitsdojo_window_configure() -> UInt {
    return BDW_CUSTOM_FRAME | BDW_HIDE_ON_STARTUP
  }

  override func awakeFromNib() {
    let flutterViewController = FlutterViewController.init()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    let windowControlButtonChannel = FlutterMethodChannel(
      name: "not.ci.rune/window_control_button",
      binaryMessenger: flutterViewController.engine.binaryMessenger)
    windowControlButtonChannel.setMethodCallHandler { (call, result) in
      switch call.method {
      case "set_vertical":
        WindowButtonPositioner.shared.setVertical()
      case "set_hide":
        WindowButtonPositioner.shared.setHide()
      case "set_show":
        WindowButtonPositioner.shared.setShow()
      default:
        result(FlutterMethodNotImplemented)
      }
    }

    RegisterGeneratedPlugins(registry: flutterViewController)

    WindowButtonPositioner.shared.mainFlutterWindow = self

    NotificationCenter.default.addObserver(
      self,
      selector: #selector(windowDidEnterFullScreen),
      name: NSWindow.didEnterFullScreenNotification,
      object: self)
            
    NotificationCenter.default.addObserver(
      self,
      selector: #selector(windowDidExitFullScreen),
      name: NSWindow.didExitFullScreenNotification,
      object: self)

    NotificationCenter.default.addObserver(
      self,
      selector: #selector(windowWillBeginSheet),
      name: NSWindow.willBeginSheetNotification,
      object: self)

    NotificationCenter.default.addObserver(
      self,
      selector: #selector(windowDidEndSheet),
      name: NSWindow.didEndSheetNotification,
      object: self)

    super.awakeFromNib()
  }

  @objc private func windowDidEnterFullScreen(_ notification: Notification) {}

  @objc private func windowDidExitFullScreen(_ notification: Notification) {
    WindowButtonPositioner.shared.setVertical()
  }

  @objc private func windowWillBeginSheet(_ notification: Notification) {
    WindowButtonPositioner.shared.setVertical()
  }

  @objc private func windowDidEndSheet(_ notification: Notification) {
    WindowButtonPositioner.shared.setVertical()
  }

  deinit {
    NotificationCenter.default.removeObserver(self)
  }
}

class WindowButtonPositioner {
  static let shared = WindowButtonPositioner()

  var mainFlutterWindow: NSWindow? = nil

  init() {}

  init(mainFlutterWindow: NSWindow) {
    self.mainFlutterWindow = mainFlutterWindow
  }

  func setHide() {
    mainFlutterWindow?.standardWindowButton(.closeButton)?.isHidden = true
    mainFlutterWindow?.standardWindowButton(.miniaturizeButton)?.isHidden = true
    mainFlutterWindow?.standardWindowButton(.zoomButton)?.isHidden = true
  }

  func setShow() {
    mainFlutterWindow?.standardWindowButton(.closeButton)?.isHidden = false
    mainFlutterWindow?.standardWindowButton(.miniaturizeButton)?.isHidden = false
    mainFlutterWindow?.standardWindowButton(.zoomButton)?.isHidden = false
  }

  func setVertical() {
    overrideStandardWindowButtonPosition(buttonType: .closeButton, offset: .init(x: 8, y: 8))
    overrideStandardWindowButtonPosition(buttonType: .miniaturizeButton, offset: .init(x: 8, y: 28))
    overrideStandardWindowButtonPosition(buttonType: .zoomButton, offset: .init(x: 8, y: 48))
  }

  func overrideStandardWindowButtonPosition(
    buttonType: NSWindow.ButtonType, offset: CGPoint
  ) {
    guard let standardWindowButton = mainFlutterWindow!.standardWindowButton(buttonType) else {
      return
    }

    standardWindowButton.removeFromSuperview()

    guard let contentView: NSView = mainFlutterWindow!.contentView else {
      return
    }

    contentView.addSubview(standardWindowButton)

    standardWindowButton.translatesAutoresizingMaskIntoConstraints = false
    standardWindowButton.wantsLayer = true

    NSLayoutConstraint.activate([
      standardWindowButton.leadingAnchor.constraint(
        equalTo: contentView.leadingAnchor, constant: offset.x),
      standardWindowButton.topAnchor.constraint(
        equalTo: contentView.topAnchor, constant: offset.y),
    ])

    contentView.layoutSubtreeIfNeeded()
  }
}
