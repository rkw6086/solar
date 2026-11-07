use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=native/main.swift");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let binary = out.join("solar-native-helper");
    let architecture = if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    let target = format!("{architecture}-apple-macos14.0");
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-parse-as-library",
            "-O",
            "-target",
            &target,
            "-framework",
            "AppKit",
            "-framework",
            "ScreenCaptureKit",
            "-framework",
            "ApplicationServices",
            "native/main.swift",
            "-o",
        ])
        .arg(&binary)
        .status()
        .expect("Xcode command-line tools are required to build the macOS native helper");
    assert!(status.success(), "macOS native helper compilation failed");
    println!(
        "cargo:rustc-env=SOLAR_NATIVE_HELPER_BUILT={}",
        binary.display()
    );
    println!("cargo:metadata=native_helper={}", binary.display());
}
