use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=native/AppLicense.swift");
    println!("cargo:rerun-if-changed=native/AppAccount.swift");
    println!("cargo:rerun-if-changed=native/VisionOcr.swift");
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("Cargo must provide target OS");
    if target_os == "macos" {
        // OCR is not a feature: every macOS build, including `cargo test`, links Vision so the
        // sandboxed App Store build can read scanned documents without external tools.
        compile_swift_bridge(&SwiftBridge {
            feature: "macOS OCR",
            source: "native/VisionOcr.swift",
            module: "OnRecordVisionOcr",
            library: "onrecord_vision_ocr",
            frameworks: &["Vision", "PDFKit", "ImageIO", "CoreGraphics", "Foundation"],
            extra_swift_flags: &[],
        });
    }
    let app_store = env::var_os("CARGO_FEATURE_APP_STORE").is_some();
    let direct_license = env::var_os("CARGO_FEATURE_DIRECT_LICENSE").is_some();
    if app_store {
        compile_swift_bridge(&SwiftBridge {
            feature: "app-store",
            source: "native/AppLicense.swift",
            module: "OnRecordStoreKit",
            library: "onrecord_storekit",
            frameworks: &["StoreKit", "Foundation"],
            extra_swift_flags: &[],
        });
    }
    if direct_license {
        // The Swift `#if DEBUG` gates in AppAccount.swift follow the Cargo profile; swiftc does not
        // know it. A release build must never compile the debugger checks out.
        let debug_profile = env::var("PROFILE").is_ok_and(|profile| profile == "debug");
        compile_swift_bridge(&SwiftBridge {
            feature: "direct-license",
            source: "native/AppAccount.swift",
            module: "OnRecordAccount",
            library: "onrecord_account",
            frameworks: &[
                "AuthenticationServices",
                "AppKit",
                "Security",
                "CryptoKit",
                "Foundation",
            ],
            extra_swift_flags: if debug_profile { &["-DDEBUG"] } else { &[] },
        });
    }
    if app_store || direct_license {
        // An executable does not need to export the statically linked DuckDB C++ API or the
        // internal Swift bridges. Keeping only the process entry point in the Mach-O export trie
        // lets Cargo's release stripping remove thousands of third-party symbol names without
        // changing Rust's link-time references or relying on runtime anti-debugging behavior.
        // The bin target is `onrecord-studio` (Cargo.toml [[bin]]); the flag names the binary, not
        // the crate, and Cargo refuses a name no bin target carries.
        println!("cargo:rustc-link-arg-bin=onrecord-studio=-Wl,-exported_symbol,_main");
    }
    tauri_build::build();
}

struct SwiftBridge {
    feature: &'static str,
    source: &'static str,
    module: &'static str,
    library: &'static str,
    frameworks: &'static [&'static str],
    extra_swift_flags: &'static [&'static str],
}

fn compile_swift_bridge(bridge: &SwiftBridge) {
    let feature = bridge.feature;
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("Cargo must provide target OS");
    if target_os != "macos" {
        panic!("The {feature} feature is supported only for macOS targets");
    }

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("Cargo must provide target arch");
    let swift_target = match target_arch.as_str() {
        "aarch64" => "arm64-apple-macosx13.0",
        "x86_64" => "x86_64-apple-macosx13.0",
        other => panic!("Unsupported {feature} target architecture: {other}"),
    };
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR"));
    let library = out_dir.join(format!("lib{}.a", bridge.library));
    let module_cache = out_dir.join("swift-module-cache");
    let source = Path::new(bridge.source);
    let sdk = command_output("xcrun", &["--sdk", "macosx", "--show-sdk-path"]);

    let status = Command::new("xcrun")
        .args([
            "--sdk",
            "macosx",
            "swiftc",
            "-parse-as-library",
            "-swift-version",
            "5",
            "-O",
            "-sdk",
            sdk.trim(),
            "-target",
            swift_target,
            "-emit-library",
            "-static",
            "-module-name",
            bridge.module,
            "-module-cache-path",
        ])
        .arg(&module_cache)
        .args(bridge.extra_swift_flags)
        .arg(source)
        .arg("-o")
        .arg(&library)
        .status()
        .unwrap_or_else(|error| panic!("failed to launch Swift compiler for {feature}: {error}"));
    if !status.success() {
        panic!("Swift bridge {} failed to compile", bridge.source);
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static={}", bridge.library);
    for framework in bridge.frameworks {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}

fn command_output(command: &str, arguments: &[&str]) -> String {
    let output = Command::new(command)
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("failed to launch {command}: {error}"));
    if !output.status.success() {
        panic!(
            "{command} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).expect("command output must be UTF-8")
}
