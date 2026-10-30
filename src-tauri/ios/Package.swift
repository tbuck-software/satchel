// swift-tools-version:5.5

import PackageDescription

// The mail notification delegate registers as a Tauri `Plugin`, so it imports
// the `Tauri` module. That module exists only inside the Rust build: tauri's
// build script links it into libapp.a, and the Xcode target `tauri ios init`
// generates cannot see it. `src-tauri/build.rs` therefore builds this package
// and links it into libapp.a as well, the way plugin crates link their Swift
// code, after copying tauri's Swift package to `../.tauri/tauri-api`.
let package = Package(
  name: "SatchelIOS",
  platforms: [
    .macOS(.v10_13),
    .iOS(.v13),
  ],
  products: [
    .library(
      name: "SatchelIOS",
      type: .static,
      targets: ["SatchelIOS"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "SatchelIOS",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources/SatchelIOS")
  ]
)
