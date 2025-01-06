import Foundation
import SwiftRs

@_cdecl("bundle_id")
public func bundle_id() -> SRString {
    return SRString(Bundle.main.bundleIdentifier ?? "")
}