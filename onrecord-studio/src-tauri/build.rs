use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=native/AppLicense.swift");
    if env::var_os("CARGO_FEATURE_APP_STORE").is_some() {
        compile_storekit_bridge();
    }
    tauri_build::build();
}

fn compile_storekit_bridge() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("Cargo must provide target OS");
    if target_os != "macos" {
        panic!("The app-store feature is supported only for macOS targets");
    }

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("Cargo must provide target arch");
    let swift_target = match target_arch.as_str() {
        "aarch64" => "arm64-apple-macosx13.0",
        "x86_64" => "x86_64-apple-macosx13.0",
        other => panic!("Unsupported Mac App Store target architecture: {other}"),
    };
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR"));
    let library = out_dir.join("libonrecord_storekit.a");
    let module_cache = out_dir.join("swift-module-cache");
    let source = Path::new("native/AppLicense.swift");
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
            "OnRecordStoreKit",
            "-module-cache-path",
        ])
        .arg(&module_cache)
        .arg(source)
        .arg("-o")
        .arg(&library)
        .status()
        .expect("failed to launch Swift compiler for StoreKit licensing");
    if !status.success() {
        panic!("Swift StoreKit licensing bridge failed to compile");
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=onrecord_storekit");
    println!("cargo:rustc-link-lib=framework=StoreKit");
    println!("cargo:rustc-link-lib=framework=Foundation");
    // An executable does not need to export the statically linked DuckDB C++ API or the internal
    // StoreKit bridge. Keeping only the process entry point in the Mach-O export trie lets Cargo's
    // release stripping remove thousands of third-party symbol names without changing Rust's
    // link-time references or relying on runtime anti-debugging behavior.
    println!("cargo:rustc-link-arg-bin=onrecord-studio-lib=-Wl,-exported_symbol,_main");
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
