import AppKit
import CoreGraphics

let app = NSApplication.shared
_ = app.setActivationPolicy(.accessory)
guard let screen = NSScreen.main?.visibleFrame else {
    fatalError("The window-list test requires a macOS desktop session")
}

func window(offset: CGFloat) -> NSWindow {
    NSWindow(
        contentRect: NSRect(x: screen.minX + 40 + offset, y: screen.minY + 40, width: 120, height: 100),
        styleMask: [.titled],
        backing: .buffered,
        defer: false
    )
}

let back = window(offset: 0)
let front = window(offset: 30)
let desktop = window(offset: 60)
desktop.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopWindow)))
desktop.orderFrontRegardless()
back.orderFrontRegardless()
front.order(.above, relativeTo: back.windowNumber)

let backID = CGWindowID(back.windowNumber)
let frontID = CGWindowID(front.windowNumber)
let desktopID = CGWindowID(desktop.windowNumber)

func contains(_ options: CGWindowListOption, relative: CGWindowID, window: CGWindowID) -> Bool {
    let list = CGWindowListCopyWindowInfo(options, relative) as? [[String: Any]] ?? []
    return list.contains { ($0[kCGWindowNumber as String] as? NSNumber)?.uint32Value == window }
}

// Wait for WindowServer to acknowledge the ordering before handing control to Rust.
let deadline = Date().addingTimeInterval(5)
while !contains(.optionOnScreenAboveWindow, relative: backID, window: frontID)
    || !contains(.optionAll, relative: kCGNullWindowID, window: desktopID)
{
    guard Date() < deadline else { fatalError("WindowServer did not list the fixture windows") }
    RunLoop.current.run(until: Date().addingTimeInterval(0.025))
}

print("\(backID) \(frontID) \(desktopID)")
fflush(stdout)
withExtendedLifetime((back, front, desktop)) {
    _ = readLine()
}
