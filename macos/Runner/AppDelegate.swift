import Cocoa
import FlutterMacOS

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
