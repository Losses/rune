// swift-tools-version: 6.3
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "apple-bridge-library",
    platforms: [
        .macOS(.v12),
        .iOS(.v15)
    ],
    products: [
        .library(name: "apple-bridge-library", type: .static, targets: ["apple-bridge-library"])
    ],
    targets: [
        .target(
            name: "apple-bridge-library",
            path: "src"
        )
    ],
    swiftLanguageModes: [.v6]
)
