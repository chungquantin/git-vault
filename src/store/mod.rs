mod file;
mod keychain;
mod libsecret;
mod op;
mod pass;

use crate::config::{Config, Resolved};
use crate::util::which;
use anyhow::{bail, Result};

pub const BACKENDS: &[&str] = &["keychain", "file", "pass", "op", "libsecret"];

/// A place secrets (tokens) live. `account` is the vault name.
pub trait Store {
    fn kind(&self) -> &'static str;
    fn get(&self, account: &str) -> Result<Option<String>>;
    fn set(&self, account: &str, secret: &str) -> Result<()>;
    fn delete(&self, account: &str) -> Result<()>;
}

pub fn default_backend() -> &'static str {
    if cfg!(target_os = "macos") {
        "keychain"
    } else if which("secret-tool").is_some() {
        "libsecret"
    } else if which("pass").is_some() {
        "pass"
    } else {
        "file"
    }
}

/// Precedence: explicit choice (flag/env) > config.toml `backend` > platform default.
pub fn open(choice: Option<&str>, cfg: &Config) -> Result<Box<dyn Store>> {
    let name = choice
        .map(str::to_string)
        .or_else(|| cfg.backend.clone())
        .unwrap_or_else(|| default_backend().to_string());
    Ok(match name.as_str() {
        "keychain" => Box::new(keychain::Keychain),
        "file" => Box::new(file::AgeFile),
        "pass" => Box::new(pass::Pass),
        "op" => Box::new(op::OnePassword),
        "libsecret" => Box::new(libsecret::LibSecret),
        other => bail!(
            "unknown backend '{other}' (choose: {})",
            BACKENDS.join(", ")
        ),
    })
}

/// Find the token for a vault, walking the `extends` chain (own token wins).
/// A parent's token is only inherited when it was issued for the same host/provider/API,
/// so `extends` can never carry a GitHub token to another host.
/// Returns (vault that owns it, token).
pub fn token_for(
    store: &dyn Store,
    cfg: &Config,
    r: &Resolved,
) -> Result<Option<(String, String)>> {
    for n in &r.chain {
        if n != &r.name {
            let owner = cfg.resolve(n)?;
            if owner.v.host() != r.v.host()
                || owner.v.provider != r.v.provider
                || owner.v.api_url != r.v.api_url
            {
                continue;
            }
        }
        if let Some(t) = store.get(n)? {
            return Ok(Some((n.clone(), t)));
        }
    }
    Ok(None)
}
