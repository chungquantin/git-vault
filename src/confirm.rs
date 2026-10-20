//! Ask the human before a token is released (aws-vault `--prompt` equivalent).
use crate::util::{data_dir, run_capture};
use anyhow::{anyhow, bail, Result};
use std::io::{BufRead, BufReader, Write};
use std::process::Command;

pub const MODES: &[&str] = &["none", "terminal", "dialog", "touchid"];

pub fn gate(mode: &str, vault: &str, reason: &str) -> Result<()> {
    match mode {
        "" | "none" => Ok(()),
        "terminal" => terminal(vault, reason),
        "dialog" => dialog(vault, reason),
        "touchid" => touchid(vault, reason),
        other => bail!(
            "unknown confirm mode '{other}' (choose: {})",
            MODES.join(", ")
        ),
    }
}

fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || " .-_/:@".contains(*c))
        .collect()
}

fn terminal(vault: &str, reason: &str) -> Result<()> {
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| anyhow!("confirm=terminal but no terminal available; use confirm=dialog"))?;
    write!(
        tty,
        "git-vault: release token for '{}' ({})? [y/N] ",
        clean(vault),
        clean(reason)
    )?;
    tty.flush()?;
    let mut line = String::new();
    BufReader::new(tty).read_line(&mut line)?;
    if matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        bail!("denied by user")
    }
}

fn dialog(vault: &str, reason: &str) -> Result<()> {
    let msg = format!("Release token for {} ({})?", clean(vault), clean(reason));
    let ok = if cfg!(target_os = "macos") {
        let script = format!(
            "display dialog \"{msg}\" with title \"git-vault\" buttons {{\"Deny\",\"Allow\"}} default button \"Deny\" cancel button \"Deny\""
        );
        run_capture(Command::new("osascript").args(["-e", &script]))?.0
    } else if crate::util::which("zenity").is_some() {
        run_capture(Command::new("zenity").args([
            "--question",
            "--title=git-vault",
            &format!("--text={msg}"),
        ]))?
        .0
    } else {
        bail!("confirm=dialog needs osascript (macOS) or zenity (Linux)");
    };
    if ok {
        Ok(())
    } else {
        bail!("denied by user")
    }
}

const TOUCHID_SWIFT: &str = r#"import LocalAuthentication
import Foundation
let c = LAContext()
var e: NSError?
guard c.canEvaluatePolicy(.deviceOwnerAuthentication, error: &e) else { exit(2) }
let s = DispatchSemaphore(value: 0)
var ok = false
c.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: CommandLine.arguments[1]) { r, _ in ok = r; s.signal() }
s.wait()
exit(ok ? 0 : 1)
"#;

/// Touch ID / password via LocalAuthentication. Compiles a tiny helper once with swiftc
/// and caches it. (Keychain-ACL based biometrics need a code-signed binary, which a
/// plain CLI install is not.)
fn touchid(vault: &str, reason: &str) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("confirm=touchid is macOS only");
    }
    let bin = data_dir().join("bin").join("touchid-v1");
    if !bin.exists() {
        let dir = bin.parent().unwrap();
        std::fs::create_dir_all(dir)?;
        let src = dir.join("touchid-v1.swift");
        std::fs::write(&src, TOUCHID_SWIFT)?;
        let (ok, _, err) = run_capture(
            Command::new("swiftc")
                .arg("-O")
                .arg("-o")
                .arg(&bin)
                .arg(&src),
        )?;
        if !ok {
            bail!(
                "could not build the Touch ID helper (install Xcode command line tools): {}",
                err.trim()
            );
        }
    }
    let why = format!(
        "release git-vault token for {} ({})",
        clean(vault),
        clean(reason)
    );
    let (ok, _, _) = run_capture(Command::new(&bin).arg(why))?;
    if ok {
        Ok(())
    } else {
        bail!("authentication failed or cancelled")
    }
}
