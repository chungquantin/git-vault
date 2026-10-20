use super::Store;
use crate::util::{run_capture, run_with_stdin, SERVICE};
use anyhow::{bail, Result};
use std::process::Command;

/// 1Password through the `op` CLI. Items are titled "git-vault <vault>".
/// Set GIT_VAULT_OP_VAULT to pick the 1Password vault.
pub struct OnePassword;

fn title(account: &str) -> String {
    format!("{SERVICE} {account}")
}

fn vault_args() -> Vec<String> {
    match std::env::var("GIT_VAULT_OP_VAULT") {
        Ok(v) => vec!["--vault".into(), v],
        Err(_) => vec![],
    }
}

/// Item ids currently carrying our title (normally zero or one).
fn ids(account: &str) -> Vec<String> {
    let Ok((ok, out, _)) = run_capture(
        Command::new("op")
            .args(["item", "list", "--format", "json"])
            .args(vault_args()),
    ) else {
        return vec![];
    };
    if !ok {
        return vec![];
    }
    let t = title(account);
    serde_json::from_str::<serde_json::Value>(&out)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter(|i| i["title"].as_str() == Some(t.as_str()))
        .filter_map(|i| i["id"].as_str().map(str::to_string))
        .collect()
}

impl Store for OnePassword {
    fn kind(&self) -> &'static str {
        "op"
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        let Some(id) = ids(account).into_iter().next() else {
            return Ok(None);
        };
        let (ok, out, _) = run_capture(
            Command::new("op")
                .args(["item", "get", &id, "--fields", "label=password", "--reveal"])
                .args(vault_args()),
        )?;
        Ok((ok && !out.trim().is_empty()).then(|| out.trim_end_matches('\n').to_string()))
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        let old = ids(account);
        // Create first: if this fails the previous token is still there.
        let tpl = serde_json::json!({
            "title": title(account),
            "category": "PASSWORD",
            "fields": [{ "id": "password", "type": "CONCEALED", "purpose": "PASSWORD", "label": "password", "value": secret }]
        })
        .to_string();
        let (ok, _, err) = run_with_stdin(
            Command::new("op")
                .args(["item", "create", "--template", "/dev/stdin"])
                .args(vault_args()),
            &tpl,
        )?;
        if !ok {
            bail!("op item create failed: {}", err.trim());
        }
        for id in old {
            let _ = run_capture(
                Command::new("op")
                    .args(["item", "delete", &id])
                    .args(vault_args()),
            );
        }
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<()> {
        for id in ids(account) {
            let _ = run_capture(
                Command::new("op")
                    .args(["item", "delete", &id])
                    .args(vault_args()),
            );
        }
        Ok(())
    }
}
