//! Injects the git revision and build date at compile time so `--version` (and
//! the web UI) can report exactly which build is running.
//!
//! Reproducible (Nix/Docker) builds have no `.git`, so both values fall back to
//! `unknown`; the Nix package passes `TIDL_GIT_SHA` explicitly, and
//! `SOURCE_DATE_EPOCH` (which Nix sets) keeps the date deterministic.

use std::process::Command;

/// Run a command and return its trimmed stdout, or `None` on any failure.
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-env-changed=TIDL_GIT_SHA");

    let sha = std::env::var("TIDL_GIT_SHA")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| run("git", &["rev-parse", "--short", "HEAD"]))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=TIDL_GIT_SHA={sha}");

    // Prefer the real commit date (dev builds); fall back to SOURCE_DATE_EPOCH
    // for reproducible builds, where `.git` is absent and Nix pins the epoch.
    let date = run("git", &["log", "-1", "--format=%cs"])
        .or_else(|| {
            std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|epoch| {
                let at = format!("@{epoch}");
                run("date", &["-u", "-d", &at, "+%Y-%m-%d"])
            })
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=TIDL_BUILD_DATE={date}");
}
