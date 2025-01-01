// swift-tools-version: 5.5
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "apple-bridge-library",
    platforms: [
        .macOS(.v10_13),
        .iOS(.v14)  // Minimum iOS version requirement
    ],
    products: [
        .library(name: "apple-bridge-library", type: .static, targets: ["apple-bridge-library"])
    ],
    dependencies: [
        .package(name: "SwiftRs", url: "https://github.com/Brendonovich/swift-rs", from: "1.0.5")
    ],
    targets: [
        .target(
            name: "apple-bridge-library",
            dependencies: [
                .product(name: "SwiftRs", package: "SwiftRs")
            ],
            path: "src"
        )
    ]
)
