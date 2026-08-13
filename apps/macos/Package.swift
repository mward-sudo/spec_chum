// swift-tools-version: 6.0
import PackageDescription

let repoRoot = "../.."
let hostLibDir = "\(repoRoot)/target/release"

let package = Package(
    name: "SpecChumMac",
    platforms: [
        .macOS(.v14),
    ],
    products: [
        .executable(name: "SpecChumMac", targets: ["SpecChumMac"]),
    ],
    targets: [
        .systemLibrary(
            name: "CSpecChumHost",
            path: "Sources/CSpecChumHost"
        ),
        .executableTarget(
            name: "SpecChumMac",
            dependencies: ["CSpecChumHost"],
            path: "Sources/SpecChumMac",
            swiftSettings: [
                .swiftLanguageMode(.v5),
            ],
            linkerSettings: [
                .linkedLibrary("spec_chum_host"),
                .unsafeFlags([
                    "-L\(hostLibDir)",
                    "-Xlinker", "-rpath",
                    "-Xlinker", hostLibDir,
                ]),
            ]
        ),
    ]
)
