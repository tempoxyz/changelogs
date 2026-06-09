// swift-tools-version: 5.10
// changelogs:version 0.4.2
import PackageDescription

let package = Package(
    name: "TempoKit",
    products: [
        .library(name: "TempoKit", targets: ["TempoKit"]),
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-log.git", from: "1.5.0"),
        .package(url: "https://github.com/apple/swift-collections.git", exact: "1.1.0"),
    ],
    targets: [
        .target(name: "TempoKit"),
    ]
)
