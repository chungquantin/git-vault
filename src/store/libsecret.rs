use super::Store;
use crate::util::{run_capture, run_with_stdin, SERVICE};
use anyhow::{bail, Result};
use std::process::Command;

/// Linux Secret Service (GNOME Keyring / KWallet) through `secret-tool`.
pub struct LibSecret;

impl Store for LibSecret {
    fn kind(&self) -> &'static str {
        "libsecret"
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        let (ok, out, _) = run_capture(
            Command::new("secret-tool").args(["lookup", "service", SERVICE, "account", account]),
        )?;
        Ok((ok && !out.is_empty()).then(|| out.trim_end_matches('\n').to_string()))
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        let label = format!("git-vault {account}");
        let (ok, _, err) = run_with_stdin(
            Command::new("secret-tool").args([
                "store", "--label", &label, "service", SERVICE, "account", account,
            ]),
            secret,
        )?;
        if !ok {
            bail!("secret-tool store failed: {}", err.trim());
        }
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<()> {
        let _ = run_capture(
            Command::new("secret-tool").args(["clear", "service", SERVICE, "account", account]),
        );
        Ok(())
    }
}
