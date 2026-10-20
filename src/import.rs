//! `import gh` (accounts known to gh) and `import ssh` (Host blocks in ~/.ssh/config).
use crate::util::{expand_tilde, home_dir};
use anyhow::Result;
use std::path::PathBuf;

/// (host, user) pairs from gh's hosts.yml (handles the multi-account `users:` layout and the legacy `user:` key).
pub fn parse_gh_hosts(yaml: &str) -> Vec<(String, String)> {
    let base = yaml
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|l| l.len() - l.trim_start().len())
        .filter(|&i| i > 0)
        .min()
        .unwrap_or(4);
    let (mut host, mut in_users, mut had_users) = (String::new(), false, false);
    let mut legacy: Option<(String, String)> = None;
    let mut out: Vec<(String, String)> = Vec::new();
    for line in yaml.lines().chain(std::iter::once("end:")) {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent == 0 && t.ends_with(':') {
            // new host block: a host without a `users:` map falls back to its legacy `user:` key
            if !had_users {
                out.extend(legacy.take());
            }
            legacy = None;
            host = t.trim_end_matches(':').to_string();
            in_users = false;
            had_users = false;
        } else if indent == base {
            in_users = t == "users:";
            had_users |= in_users;
            if let Some(u) = t.strip_prefix("user:") {
                legacy = Some((host.clone(), u.trim().to_string()));
            }
        } else if indent == base * 2 && in_users && t.ends_with(':') {
            out.push((host.clone(), t.trim_end_matches(':').to_string()));
        }
    }
    out
}

pub fn gh_hosts_path() -> PathBuf {
    match std::env::var_os("GH_CONFIG_DIR") {
        Some(d) => PathBuf::from(d).join("hosts.yml"),
        None => home_dir().join(".config").join("gh").join("hosts.yml"),
    }
}

#[derive(Debug, PartialEq)]
pub struct SshHost {
    pub alias: String,
    pub hostname: String,
    pub identity: String,
}

/// Concrete `Host` blocks that have both HostName and IdentityFile.
pub fn parse_ssh_config(text: &str) -> Vec<SshHost> {
    let mut out = Vec::new();
    let (mut alias, mut hostname, mut identity) = (None::<String>, None::<String>, None::<String>);
    let mut push = |a: &mut Option<String>, h: &mut Option<String>, i: &mut Option<String>| {
        if let (Some(a), Some(h), Some(i)) = (a.take(), h.take(), i.take()) {
            out.push(SshHost {
                alias: a,
                hostname: h,
                identity: expand_tilde(&i),
            });
        }
        *h = None;
        *i = None;
    };
    for line in text.lines() {
        let t = line.trim();
        let mut it = t.splitn(2, char::is_whitespace);
        let (k, v) = (
            it.next().unwrap_or("").to_lowercase(),
            it.next().unwrap_or("").trim().trim_matches('"').to_string(),
        );
        match k.as_str() {
            "host" => {
                push(&mut alias, &mut hostname, &mut identity);
                let first = v.split_whitespace().next().unwrap_or("");
                alias = (!first.contains(['*', '?', '!']) && !first.is_empty())
                    .then(|| first.to_string());
            }
            "hostname" => hostname = Some(v),
            "identityfile" if identity.is_none() => identity = Some(v),
            _ => {}
        }
    }
    push(&mut alias, &mut hostname, &mut identity);
    out
}

pub fn ssh_config_path() -> PathBuf {
    home_dir().join(".ssh").join("config")
}

pub fn read(p: &PathBuf) -> Result<String> {
    Ok(std::fs::read_to_string(p)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gh_multi_account() {
        let y = "github.com:\n    git_protocol: https\n    users:\n        alice:\n            oauth_token: x\n        bob:\n    user: alice\ngit.corp.io:\n    users:\n        carol:\n";
        assert_eq!(
            parse_gh_hosts(y),
            vec![
                ("github.com".into(), "alice".into()),
                ("github.com".into(), "bob".into()),
                ("git.corp.io".into(), "carol".into())
            ]
        );
    }

    #[test]
    fn gh_legacy_single_user() {
        let y = "github.com:\n    oauth_token: x\n    user: dave\n    git_protocol: https\n";
        assert_eq!(
            parse_gh_hosts(y),
            vec![("github.com".into(), "dave".into())]
        );
    }

    #[test]
    fn ssh_hosts() {
        let c = "Host *\n  AddKeysToAgent yes\n\nHost gh-work\n  HostName github.com\n  IdentityFile ~/.ssh/id_work\n  IdentitiesOnly yes\n\nHost gh-me\n  HostName github.com\n  IdentityFile /k/id_me\nHost nokey\n  HostName x.io\n";
        let h = parse_ssh_config(c);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].alias, "gh-work");
        assert!(h[0].identity.ends_with("/.ssh/id_work"));
        assert_eq!(
            h[1],
            SshHost {
                alias: "gh-me".into(),
                hostname: "github.com".into(),
                identity: "/k/id_me".into()
            }
        );
    }
}
