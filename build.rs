use std::process::Command;

/// Best-effort git revision for the version string. Falls back to "unknown"
/// when git is unavailable or the source is built outside a git repository
/// (e.g. when installed from crates.io), so published builds never fail here.
fn git_sha() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let sha = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if sha.is_empty() {
        return None;
    }

    // Mark the build dirty if there are uncommitted changes.
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);

    Some(format!("{}{}", sha, if dirty { "-dirty" } else { "" }))
}

fn main() {
    let git_sha = git_sha().unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=GIT_SHA={}", git_sha);
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
