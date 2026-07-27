// swift-tools-version: 6.3
import PackageDescription

let package = Package(
    name: "AlphaDesk",
    platforms: [.macOS(.v15), .iOS(.v18)],
    products: [
        .library(name: "DeskDomain", targets: ["DeskDomain"]),
        .library(name: "DeskNetworking", targets: ["DeskNetworking"]),
        .library(name: "DeskStorage", targets: ["DeskStorage"]),
    ],
    targets: [
        .target(name: "DeskDomain"),
        .target(name: "DeskNetworking", dependencies: ["DeskDomain"]),
        .target(name: "DeskStorage", dependencies: ["DeskDomain"]),
        .testTarget(name: "DeskDomainTests", dependencies: ["DeskDomain"]),
    ],
    swiftLanguageModes: [.v6]
)
