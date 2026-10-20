use super::Store;
use crate::util::{hex, run_capture, run_with_stdin, SERVICE};
use anyhow::{bail, Result};
use std::process::Command;

/// macOS Keychain through the `security` CLI.
pub struct Keychain;

impl Store for Keychain {
    fn kind(&self) -> &'static str {
        "keychain"
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        if !cfg!(target_os = "macos") {
            bail!("keychain backend is macOS only; use --backend file|pass|libsecret|op");
        }
        let (ok, out, _) = run_capture(Command::new("security").args([
            "find-generic-password",
            "-a",
            account,
            "-s",
            SERVICE,
            "-w",
        ]))?;
        Ok(ok.then(|| out.trim_end_matches('\n').to_string()))
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        if !cfg!(target_os = "macos") {
            bail!("keychain backend is macOS only");
        }
        if !crate::util::valid_name(account) {
            bail!("invalid vault name {account:?}");
        }
        // Feed the command on stdin (`security -i`) with the secret hex-encoded,
        // so it never appears in argv / `ps`.
        let line = format!(
            "add-generic-password -a \"{account}\" -s \"{SERVICE}\" -U -X {}\n",
            hex(secret.as_bytes())
        );
        run_with_stdin(Command::new("security").arg("-i"), &line)?;
        // `security -i` swallows errors into its exit code, so verify.
        match self.get(account)? {
            Some(s) if s == secret => Ok(()),
            _ => bail!("keychain write failed"),
        }
    }

    fn delete(&self, account: &str) -> Result<()> {
        let _ = run_capture(Command::new("security").args([
            "delete-generic-password",
            "-a",
            account,
            "-s",
            SERVICE,
        ]));
        Ok(())
    }
}
