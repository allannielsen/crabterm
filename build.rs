use std::process::Command;

fn main() {
    // Get git SHA
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .expect("Failed to execute git");

    let git_sha = String::from_utf8(output.stdout).unwrap().trim().to_string();

    // Check if working directory is dirty
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .expect("Failed to execute git");

    let dirty = if status.stdout.is_empty() {
        ""
    } else {
        "-dirty"
    };

    println!("cargo:rustc-env=GIT_SHA={}{}", git_sha, dirty);
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
