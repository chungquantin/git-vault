use super::Store;
use crate::util::{data_dir, prompt_secret};
use age::secrecy::SecretString;
use anyhow::{anyhow, bail, Context, Result};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::OnceLock;

/// Tokens as age (scrypt passphrase) encrypted files under <data>/secrets/.
/// Passphrase: GIT_VAULT_PASSPHRASE, else a terminal prompt (asked once per run).
pub struct AgeFile;

static PASS: OnceLock<String> = OnceLock::new();

fn passphrase() -> Result<String> {
    if let Some(p) = PASS.get() {
        return Ok(p.clone());
    }
    let p = match std::env::var("GIT_VAULT_PASSPHRASE") {
        Ok(p) => p,
        Err(_) => prompt_secret("git-vault file backend passphrase: ")?,
    };
    let _ = PASS.set(p.clone());
    Ok(p)
}

fn log_n() -> u8 {
    std::env::var("GIT_VAULT_SCRYPT_LOG_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(16)
}

fn path(account: &str) -> Result<PathBuf> {
    if !crate::util::valid_name(account) {
        bail!("invalid vault name {account:?}");
    }
    Ok(data_dir().join("secrets").join(format!("{account}.age")))
}

impl Store for AgeFile {
    fn kind(&self) -> &'static str {
        "file"
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        let p = path(account)?;
        if !p.exists() {
            return Ok(None);
        }
        let ct = std::fs::read(&p)?;
        let mut id = age::scrypt::Identity::new(SecretString::from(passphrase()?));
        id.set_max_work_factor(22);
        let pt = age::decrypt(&id, &ct)
            .map_err(|e| anyhow!("decrypt {}: {e} (wrong passphrase?)", p.display()))?;
        Ok(Some(String::from_utf8(pt).context("secret is not utf-8")?))
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        let mut rec = age::scrypt::Recipient::new(SecretString::from(passphrase()?));
        rec.set_work_factor(log_n());
        let ct = age::encrypt(&rec, secret.as_bytes()).map_err(|e| anyhow!("encrypt: {e}"))?;
        let p = path(account)?;
        let dir = p.parent().unwrap();
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).ok();
        // write-then-rename: a crash can never leave a truncated secret
        use std::io::Write;
        let tmp = p.with_extension("age.tmp");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(&ct)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &p)?;
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<()> {
        match std::fs::remove_file(path(account)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => bail!(e),
        }
    }
}
