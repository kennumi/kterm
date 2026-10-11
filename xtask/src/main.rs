// `cargo release [patch|minor|major]`: runs installer/release.ps1.
use std::process::{Command, exit};

fn main() {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../installer/release.ps1");
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script]);
    if let Some(bump) = std::env::args().nth(1) {
        cmd.args(["-Bump", &bump]);
    }
    exit(cmd.status().map_or(1, |s| s.code().unwrap_or(1)));
}
