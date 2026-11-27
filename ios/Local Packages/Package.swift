// swift-tools-version: 5.4
// The swift-tools-version declares the minimum version of Swift required to build this package.

import Foundation
import PackageDescription

let packageDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let hasHiddifyRust = FileManager.default.fileExists(
    atPath: packageDir.appendingPathComponent("../Frameworks/HiddifyRust.xcframework").path
)

var products: [Product] = [
    .library(
        name: "HiddifyCore",
        targets: ["HiddifyCore"]
    ),
]

var targets: [Target] = [
    .binaryTarget(
        name: "HiddifyCore",
        path: "../Frameworks/HiddifyCore.xcframework"
    ),
]

if hasHiddifyRust {
    products.append(
        .library(
            name: "HiddifyRust",
            targets: ["HiddifyRust"]
        )
    )
    targets.append(
        .binaryTarget(
            name: "HiddifyRust",
            path: "../Frameworks/HiddifyRust.xcframework"
        )
    )
}

let package = Package(
    name: "Hiddify Packages",
    platforms: [
        // Minimum platform version
        .iOS(.v13)
    ],
    products: products,
    dependencies: [
        // No dependencies
    ],
    targets: targets
)
