// swift-tools-version:5.3
import PackageDescription

let package = Package(
  name: "KiwixMobile",
  // ActivityKit (download Live Activity) is iOS 16.1+; the application's own
  // deployment target is 16.4, so the plugin package matches it.
  platforms: [.iOS("16.4")],
  products: [.library(name: "tauri-plugin-kiwix-mobile", type: .static, targets: ["KiwixMobile"])],
  dependencies: [.package(name: "Tauri", path: "../.tauri/tauri-api")],
  targets: [
    .target(name: "PdfExportBridge", path: "PdfExportBridge", publicHeadersPath: "include"),
    .target(name: "KiwixMobile", dependencies: [.byName(name: "Tauri"), .byName(name: "PdfExportBridge")], path: "Sources",
      linkerSettings: [.linkedFramework("CoreBluetooth"), .linkedFramework("NetworkExtension")]),
    .testTarget(name: "KiwixMobileTests", dependencies: ["KiwixMobile"], path: "Tests")
  ]
)
