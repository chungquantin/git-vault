//! Turn a vault into the `-c` overrides / env that make one git command act as it.
use crate::config::Resolved;
use crate::util::expand_tilde;
use anyhow::Result;
use std::path::Path;

pub struct Prepared {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

fn looks_like_gpg_id(k: &str) -> bool {
    (8..=40).contains(&k.len()) && k.chars().all(|c| c.is_ascii_hexdigit())
}

/// Identity + signing as git config key/values (used for `-c` and for `use`).
pub fn identity_pairs(r: &Resolved) -> Vec<(String, String)> {
    let mut p = Vec::new();
    if let Some(n) = &r.v.user_name {
        p.push(("user.name".into(), n.clone()));
    }
    if let Some(e) = &r.v.email {
        p.push(("user.email".into(), e.clone()));
    }
    let signing_wanted = r.v.sign.unwrap_or(false) || r.v.signing_key.is_some();
    if signing_wanted && r.v.sign != Some(false) {
        let key = r.v.signing_key.clone().or_else(|| {
            r.v.ssh_key.as_ref().map(|k| {
                let pubk = format!("{k}.pub");
                if Path::new(&pubk).exists() {
                    pubk
                } else {
                    k.clone()
                }
            })
        });
        if let Some(key) = key {
            let fmt = r.v.signing_format.clone().unwrap_or_else(|| {
                if looks_like_gpg_id(&key) {
                    "openpgp".into()
                } else {
                    "ssh".into()
                }
            });
            p.push(("gpg.format".into(), fmt));
            p.push(("user.signingkey".into(), expand_tilde(&key)));
            p.push(("commit.gpgsign".into(), "true".into()));
        }
    }
    p
}

pub fn ssh_command(r: &Resolved) -> Option<String> {
    r.v.ssh_key.as_ref().map(|k| {
        format!(
            "ssh -i {} -o IdentitiesOnly=yes",
            shlex::try_quote(k)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| k.clone())
        )
    })
}

pub fn helper_command(vault: &str) -> Result<String> {
    let exe = crate::util::stable_exe()?;
    let exe = exe.to_string_lossy();
    let q = shlex::try_quote(&exe)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| exe.to_string());
    let v = shlex::try_quote(vault)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| vault.to_string());
    Ok(format!("!{q} credential --vault {v}"))
}

/// Overrides for one invocation: identity, signing, credential helper, ssh key.
pub fn prepare(r: &Resolved) -> Result<Prepared> {
    let mut args = Vec::new();
    for (k, v) in identity_pairs(r) {
        args.push("-c".into());
        args.push(format!("{k}={v}"));
    }
    args.extend(["-c".into(), "credential.helper=".into()]);
    args.push("-c".into());
    args.push(format!("credential.helper={}", helper_command(&r.name)?));
    let mut env = vec![("GIT_VAULT_ACTIVE".to_string(), r.name.clone())];
    if let Some(s) = ssh_command(r) {
        env.push(("GIT_SSH_COMMAND".into(), s));
    }
    Ok(Prepared { args, env })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Vault;

    fn r(v: Vault) -> Resolved {
        Resolved {
            name: "t".into(),
            chain: vec!["t".into()],
            v,
        }
    }

    #[test]
    fn ssh_signing_defaults_to_pub_key() {
        let p = identity_pairs(&r(Vault {
            email: Some("a@b".into()),
            ssh_key: Some("/nonexistent/id_x".into()),
            sign: Some(true),
            ..Default::default()
        }));
        assert!(p.contains(&("gpg.format".into(), "ssh".into())));
        assert!(p.contains(&("commit.gpgsign".into(), "true".into())));
        assert!(p.contains(&("user.signingkey".into(), "/nonexistent/id_x".into())));
    }

    #[test]
    fn gpg_id_uses_openpgp() {
        let p = identity_pairs(&r(Vault {
            signing_key: Some("ABCDEF1234567890".into()),
            ..Default::default()
        }));
        assert!(p.contains(&("gpg.format".into(), "openpgp".into())));
    }

    #[test]
    fn no_signing_when_disabled() {
        let p = identity_pairs(&r(Vault {
            signing_key: Some("ABCDEF12".into()),
            sign: Some(false),
            ..Default::default()
        }));
        assert!(!p.iter().any(|(k, _)| k == "commit.gpgsign"));
    }
}
