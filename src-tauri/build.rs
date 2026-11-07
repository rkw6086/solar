fn main() {
    println!("cargo:rerun-if-env-changed=SOLAR_SOURCE_COMMIT");
    println!("cargo:rerun-if-env-changed=SOLAR_SOURCE_MODIFIED");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    for path in [".git/HEAD", ".git/index", ".git/refs/heads"] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|value| value.trim().to_owned())
    };
    if let Some(commit) = std::env::var("SOLAR_SOURCE_COMMIT")
        .ok()
        .or_else(|| git(&["rev-parse", "HEAD"]))
    {
        println!("cargo:rustc-env=SOLAR_SOURCE_COMMIT={commit}");
    }
    if let Some(modified) = std::env::var("SOLAR_SOURCE_MODIFIED")
        .ok()
        .or_else(|| git(&["status", "--porcelain"]).map(|value| (!value.is_empty()).to_string()))
    {
        println!("cargo:rustc-env=SOLAR_SOURCE_MODIFIED={modified}");
    }
    #[cfg(feature = "desktop")]
    tauri_build::build();
}
