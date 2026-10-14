fn main() {
    use std::process::Command;
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../src");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");
    let revision = Command::new("git").args(["rev-parse", "--short=12", "HEAD"])
        .output().ok().filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = Command::new("git").args(["diff", "--quiet", "HEAD", "--", ".", "../src"])
        .status().map(|status| !status.success()).unwrap_or(true);
    let built_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_secs();
    println!("cargo:rustc-env=PDF2TEST_BUILD_REVISION={revision}");
    println!("cargo:rustc-env=PDF2TEST_BUILD_DIRTY={dirty}");
    println!("cargo:rustc-env=PDF2TEST_BUILD_TIME={built_at}");
    tauri_build::build()
}
