// swift-tools-version: 5.5
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "apple-bridge-library",
    products: [
        .library(name: "apple-bridge-library", type: .static, targets: ["apple-bridge-library"]),
    ],
    dependencies: [
    ],
    targets: [
        .target(
            name: "apple-bridge-library",
            dependencies: []
        )
    ]
)
