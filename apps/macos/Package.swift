// swift-tools-version: 6.0
import PackageDescription

let repoRoot = "../.."
let hostLibDir = "\(repoRoot)/target/release"
// Bevy must be force-loaded from the static archive — a living_room cdylib
// panics / paints black when dlopened next to host_api (duplicate Rust/Bevy state).
// Embed builds use `cargo build -p living_room --release --no-default-features`
// so standalone Bevy chrome / cpal / rfd are not pulled in; host_api symbols
// still live in this archive for SpecChumMac.
let roomStatic = "\(hostLibDir)/libspec_chum_room.a"

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
                .linkedFramework("GameController"),
                .linkedFramework("Metal"),
                .linkedFramework("QuartzCore"),
                .linkedFramework("AudioToolbox"),
                .linkedFramework("CoreAudio"),
                .linkedFramework("CoreVideo"),
                .linkedFramework("IOKit"),
                .linkedFramework("Carbon"),
                // host_api lives inside libspec_chum_room.a (living_room depends on it).
                .unsafeFlags([
                    "-L\(hostLibDir)",
                    "-Xlinker", "-force_load",
                    "-Xlinker", roomStatic,
                    "-lc++",
                ]),
            ]
        ),
    ]
)
