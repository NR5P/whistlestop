//! Puts the git build the app was made from into it, so a tester's saved Log
//! names the exact build (the Log tab shows it beside the version): the beta
//! tag when built from one (`v0.1.0-beta.2`), else the last tag, commits
//! since and commit (`v0.1.0-beta.1-3-g9e7faad1c`), or just the commit;
//! `-dirty` when built with uncommitted changes. Without git, "unknown".

fn main() {
    let build = std::process::Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty", "--abbrev=9", "--match", "v*"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=WHISTLESTOP_COMMIT={build}");
}
