import Foundation

func bundle_id() -> String {
    return Bundle.main.bundleIdentifier ?? ""
}