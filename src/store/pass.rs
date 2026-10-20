use super::Store;
use crate::util::{run_capture, run_with_stdin, SERVICE};
use anyhow::{bail, Result};
use std::process::Command;

/// passwordstore.org (`pass`), entries named git-vault/<vault>.
pub struct Pass;

fn entry(account: &str) -> String {
    format!("{SERVICE}/{account}")
}

impl Store for Pass {
    fn kind(&self) -> &'static str {
        "pass"
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        let (ok, out, _) = run_capture(Command::new("pass").args(["show", &entry(account)]))?;
        Ok(ok.then(|| out.lines().next().unwrap_or("").to_string()))
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        let (ok, _, err) = run_with_stdin(
            Command::new("pass").args(["insert", "-m", "-f", &entry(account)]),
            &format!("{secret}\n"),
        )?;
        if !ok {
            bail!("pass insert failed: {}", err.trim());
        }
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<()> {
        let _ = run_capture(Command::new("pass").args(["rm", "-f", &entry(account)]));
        Ok(())
    }
}
