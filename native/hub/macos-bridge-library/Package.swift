// swift-tools-version: 5.5
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "macos-bridge-library",
    products: [
        .library(name: "macos-bridge-library", type: .static, targets: ["macos-bridge-library"]),
    ],
    dependencies: [
    ],
    targets: [
        .target(
            name: "macos-bridge-library",
            dependencies: []
        )
    ]
)
