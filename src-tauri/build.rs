// SPDX-License-Identifier: GPL-3.0-or-later
use std::env;
#[path = "native/mobile_sdk.rs"]
mod mobile_sdk;
#[path = "native/desktop_sdk.rs"]
mod desktop_sdk;

fn main() {
    // Platforms with an app-owned location provider (see permissions.rs). An
    // archive asks Kiwix, never the webview's own geolocation.
    println!("cargo::rustc-check-cfg=cfg(native_location)");
    if matches!(
        env::var("CARGO_CFG_TARGET_OS").as_deref(),
        // Linux (GeoClue) and Android providers are written but not yet
        // compiled; add "linux" | "android" here once they build and pass review.
        Ok("macos" | "ios" | "windows")
    ) {
        println!("cargo:rustc-cfg=native_location");
    }
    // The authenticated headless preview embeds this directory independently of
    // Tauri's platform-specific frontendDist. Rebuild it when those assets change.
    println!("cargo:rerun-if-changed=../dist");
    println!("cargo:rerun-if-changed=native/archive.cpp");
    println!("cargo:rerun-if-changed=native/archive.h");
    println!("cargo:rerun-if-env-changed=LIBZIM_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=LIBZIM_LIB_DIR");
    println!("cargo:rerun-if-changed=native/mobile_sdk.rs");
    println!("cargo:rerun-if-changed=native/desktop_sdk.rs");
    println!("cargo:rerun-if-env-changed=KIWIX_TYPECHECK_ONLY");

    // CI type-checks targets whose matched SDK is not provisioned (Android)
    // with `cargo clippy`. This mode skips the libzim bridge, SDK verification
    // and Objective-C sources. It never substitutes an unverified library, and
    // it requests a library that does not exist, so `cargo check`/`clippy`
    // work but every real link (binary, cdylib, tests) fails at build time
    // instead of producing an app with unresolved bridge symbols.
    if env::var("KIWIX_TYPECHECK_ONLY").as_deref() == Ok("1") {
        println!("cargo:rustc-link-lib=dylib=kiwix_typecheck_only_cannot_link");
        tauri_build_with_windows_manifest();
        return;
    }

    let mut native = cc::Build::new();
    native.cpp(true).std("c++17").file("native/archive.cpp");
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        native.flag("/EHsc");
    }
    let mobile_sdk = mobile_sdk::MobileSdk::load();

    let desktop_sdk = if let Some(sdk) = &mobile_sdk {
        assert!(env::var_os("LIBZIM_INCLUDE_DIR").is_none() && env::var_os("LIBZIM_LIB_DIR").is_none(),
            "Mobile targets use TAURI_KIWIX_MOBILE_SDK_ROOT, not desktop LIBZIM overrides");
        sdk.configure(&mut native);
        None
    } else {
        let sdk = desktop_sdk::DesktopSdk::load();
        sdk.configure(&mut native);
        Some(sdk)
    };

    native.compile("kiwix_archive");

    // The bridge precedes its dependency closure. Mobile SDKs supply only
    // target-built static archives; desktop deployment bundles dynamic runtimes.
    if let Some(sdk) = mobile_sdk {
        sdk.emit_links();
    } else {
        desktop_sdk.expect("Desktop SDK is required for non-mobile targets").emit_links();
    }

    if matches!(env::var("CARGO_CFG_TARGET_OS").as_deref(), Ok("macos" | "ios")) {
        println!("cargo:rerun-if-changed=native/location.mm");
        cc::Build::new()
            .cpp(true)
            .std("c++17")
            .file("native/location.mm")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("kiwix_location");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreLocation");
        println!("cargo:rerun-if-changed=native/notifications.mm");
        cc::Build::new()
            .cpp(true)
            .std("c++17")
            .file("native/notifications.mm")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("kiwix_notifications");
        println!("cargo:rustc-link-lib=framework=UserNotifications");
        let application_framework = if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") { "UIKit" } else { "AppKit" };
        println!("cargo:rustc-link-lib=framework={application_framework}");
        println!("cargo:rerun-if-changed=native/localswarm.mm");
        cc::Build::new()
            .cpp(true)
            .std("c++17")
            .file("native/localswarm.mm")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("kiwix_localswarm");
        println!("cargo:rustc-link-lib=framework=Network");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rerun-if-changed=native/pdf.mm");
        cc::Build::new()
            .cpp(true)
            .std("c++17")
            .file("native/pdf.mm")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("kiwix_pdf");
        println!("cargo:rustc-link-lib=framework=WebKit");
        // WebKit's under-page background is the one reader colour the page
        // cannot set: it must be right before a document exists.
        println!("cargo:rerun-if-changed=native/webview_backdrop.mm");
        cc::Build::new()
            .cpp(true)
            .std("c++17")
            .file("native/webview_backdrop.mm")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("kiwix_webview_backdrop");
        if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
            // The real OS print panel is macOS-only. iOS shares the renderer
            // but has no print button upstream (the share sheet is the only
            // system handoff), so this file is not compiled for iOS.
            println!("cargo:rerun-if-changed=native/print.mm");
            cc::Build::new()
                .cpp(true)
                .std("c++17")
                .file("native/print.mm")
                .flag("-fobjc-arc")
                .flag("-fblocks")
                .compile("kiwix_print");
            println!("cargo:rustc-link-lib=framework=PDFKit");
        }
        if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
            println!("cargo:rustc-link-lib=framework=ApplicationServices");
            println!("cargo:rerun-if-changed=native/share.mm");
            cc::Build::new()
                .cpp(true)
                .std("c++17")
                .file("native/share.mm")
                .flag("-fobjc-arc")
                .flag("-fblocks")
                .compile("kiwix_share");
            println!("cargo:rerun-if-changed=native/menu_keys.mm");
            cc::Build::new()
                .cpp(true)
                .std("c++17")
                .file("native/menu_keys.mm")
                .flag("-fobjc-arc")
                .flag("-fblocks")
                .compile("kiwix_menu_keys");
            println!("cargo:rerun-if-changed=native/tabbing.mm");
            cc::Build::new()
                .cpp(true)
                .std("c++17")
                .file("native/tabbing.mm")
                .flag("-fobjc-arc")
                .flag("-fblocks")
                .compile("kiwix_tabbing");
        }
    }

    tauri_build_with_windows_manifest();
}

/// tauri-build embeds its Common-Controls v6 manifest only into the app
/// binary. `cargo test` binaries import comctl32 v6 APIs too (the dialog
/// plugin's `TaskDialogIndirect`) and fail to start with
/// STATUS_ENTRYPOINT_NOT_FOUND without it. On MSVC, link the identical
/// manifest (a verbatim copy of tauri-build's default) into every binary,
/// tests included, and stop tauri-build from embedding a second copy.
/// link.exe merges in its default UAC fragment (`requestedExecutionLevel
/// asInvoker`), so binaries now also declare that explicitly; this only
/// disables installer-detection heuristics and changes no privilege.
fn tauri_build_with_windows_manifest() {
    let msvc_windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !msvc_windows {
        tauri_build::build();
        return;
    }
    let manifest = std::path::PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"))
        .join("windows-app-manifest.xml");
    assert!(manifest.is_file(), "Missing {}", manifest.display());
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
    )
    .expect("tauri-build failed");
}
