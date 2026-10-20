use crate::util::{data_dir, expand_tilde};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

/// One named identity/credential. Every field is optional so a vault can
/// `extends` another and override only what differs (aws-vault `source_profile`).
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Vault {
    pub extends: Option<String>,
    pub user_name: Option<String>,
    pub email: Option<String>,
    pub host: Option<String>,
    pub provider: Option<String>,
    pub username: Option<String>,
    pub api_url: Option<String>,
    pub client_id: Option<String>,
    pub ssh_key: Option<String>,
    pub signing_key: Option<String>,
    pub signing_format: Option<String>,
    pub sign: Option<bool>,
    pub confirm: Option<String>,
    pub token_name: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Rule {
    #[serde(rename = "match")]
    pub pattern: String,
    pub vault: String,
}

/// What `install-helper` changed, so `uninstall-helper` can restore it exactly and the
/// credential helper can delegate to the helpers it replaced.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct HelperState {
    pub fallbacks: Vec<String>,
    pub original_global: Vec<String>,
    pub use_http_path: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Config {
    pub backend: Option<String>,
    pub helper: Option<HelperState>,
    #[serde(default)]
    pub vaults: BTreeMap<String, Vault>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

/// A vault with its `extends` chain folded in.
#[derive(Clone, Debug)]
pub struct Resolved {
    pub name: String,
    /// Leaf first: [name, parent, grandparent, ...]
    pub chain: Vec<String>,
    pub v: Vault,
}

macro_rules! overlay {
    ($base:expr, $top:expr, $($f:ident),+) => {
        $( if $top.$f.is_some() { $base.$f = $top.$f.clone(); } )+
    };
}

impl Vault {
    pub fn host(&self) -> String {
        self.host.clone().unwrap_or_else(|| "github.com".into())
    }
    pub fn confirm_mode(&self) -> String {
        std::env::var("GIT_VAULT_CONFIRM")
            .ok()
            .or_else(|| self.confirm.clone())
            .unwrap_or_else(|| "none".into())
    }
}

impl Config {
    pub fn resolve(&self, name: &str) -> Result<Resolved> {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut cur = Some(name.to_string());
        while let Some(n) = cur {
            if !seen.insert(n.clone()) {
                bail!("`extends` cycle through vault '{n}'");
            }
            let v = self
                .vaults
                .get(&n)
                .ok_or_else(|| anyhow!("no such vault: {n} (try `git-vault list`)"))?;
            cur = v.extends.clone();
            chain.push(n);
        }
        let mut merged = Vault::default();
        for n in chain.iter().rev() {
            let v = &self.vaults[n];
            overlay!(
                merged,
                v,
                user_name,
                email,
                host,
                provider,
                username,
                api_url,
                client_id,
                ssh_key,
                signing_key,
                signing_format,
                sign,
                confirm
            );
        }
        Ok(Resolved {
            name: name.to_string(),
            chain,
            v: merged,
        })
    }

    pub fn dependents(&self, name: &str) -> Vec<String> {
        self.vaults
            .iter()
            .filter(|(_, v)| v.extends.as_deref() == Some(name))
            .map(|(k, _)| k.clone())
            .collect()
    }
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

pub fn load() -> Result<Config> {
    migrate_legacy()?;
    let p = config_path();
    if !p.exists() {
        return Ok(Config::default());
    }
    let s = std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
    let cfg: Config = toml::from_str(&s).with_context(|| format!("parsing {}", p.display()))?;
    for n in cfg.vaults.keys().chain(cfg.rules.iter().map(|r| &r.vault)) {
        if !crate::util::valid_name(n) {
            bail!(
                "invalid vault name {n:?} in {} (allowed: letters, digits, . _ -)",
                p.display()
            );
        }
    }
    Ok(cfg)
}

pub fn save(cfg: &Config) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok();
    let p = config_path();
    let tmp = p.with_extension("toml.tmp");
    let body = toml::to_string_pretty(cfg).context("serializing config")?;
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(body.as_bytes())?;
    }
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

/// v0.1 (python) kept metadata in vaults.json. Convert once; keep a .migrated copy.
fn migrate_legacy() -> Result<()> {
    let json = data_dir().join("vaults.json");
    if config_path().exists() || !json.exists() {
        return Ok(());
    }
    let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&json)?)?;
    let mut cfg = Config::default();
    if let Some(map) = raw.as_object() {
        for (name, v) in map {
            if !crate::util::valid_name(name) {
                bail!("cannot migrate vaults.json: invalid vault name {name:?}");
            }
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
            cfg.vaults.insert(
                name.clone(),
                Vault {
                    user_name: s("user_name"),
                    email: s("email"),
                    host: s("host"),
                    username: s("username"),
                    ssh_key: s("ssh_key").map(|k| expand_tilde(&k)),
                    signing_key: s("signing_key"),
                    ..Default::default()
                },
            );
        }
    }
    save(&cfg)?;
    std::fs::rename(&json, data_dir().join("vaults.json.migrated"))?;
    eprintln!("git-vault: migrated vaults.json -> config.toml");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        let mut c = Config::default();
        c.vaults.insert(
            "work".into(),
            Vault {
                email: Some("w@corp".into()),
                host: Some("github.com".into()),
                user_name: Some("W".into()),
                ..Default::default()
            },
        );
        c.vaults.insert(
            "work-ro".into(),
            Vault {
                extends: Some("work".into()),
                user_name: Some("W RO".into()),
                ..Default::default()
            },
        );
        c
    }

    #[test]
    fn inherits_and_overrides() {
        let r = cfg().resolve("work-ro").unwrap();
        assert_eq!(r.v.email.as_deref(), Some("w@corp"));
        assert_eq!(r.v.user_name.as_deref(), Some("W RO"));
        assert_eq!(r.chain, vec!["work-ro", "work"]);
    }

    #[test]
    fn detects_cycles() {
        let mut c = cfg();
        c.vaults.get_mut("work").unwrap().extends = Some("work-ro".into());
        assert!(c.resolve("work-ro").is_err());
    }

    #[test]
    fn toml_roundtrip_skips_none() {
        let s = toml::to_string_pretty(&cfg()).unwrap();
        assert!(!s.contains("ssh_key"));
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.vaults.len(), 2);
    }
}
