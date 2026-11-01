use crate::config::{Config, HelperEntry, HelperState};
use crate::util::{git_global, run_capture, run_with_stdin};
use anyhow::{bail, Result};
use std::process::Command;

pub const SHELL_INIT: &str = r#"# git-vault shell integration
export GIT_VAULT_SHELL=1
git() {
  if [ "$1" = add ] && [ "$2" = vault ]; then
    shift 2
    command git-vault add "$@"
    return
  fi
  local _a
  for _a in "$@"; do
    case "$_a" in
      --vault|--vault=*) command git-vault run "$@"; return ;;
    esac
  done
  if [ -n "${GIT_VAULT:-}" ]; then
    command git-vault run "$@"
    return
  fi
  command git "$@"
}
"#;

fn is_ours(v: &str) -> bool {
    v.contains("git-vault") && v.contains("credential")
}

fn all(scope: &str) -> Vec<String> {
    let mut c = Command::new("git");
    c.args(["config", scope, "--get-all", "credential.helper"]);
    run_capture(&mut c)
        .map(|(_, o, _)| o.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// All `credential[.<url>].helper` entries of one config scope, in file order.
fn entries(scope: &str) -> Vec<HelperEntry> {
    let mut c = Command::new("git");
    c.args([
        "config",
        scope,
        "--get-regexp",
        r"^credential(\..+)?\.helper$",
    ]);
    let out = run_capture(&mut c).map(|(_, o, _)| o).unwrap_or_default();
    out.lines()
        .filter_map(|l| {
            let (key, val) = l.split_once(' ').unwrap_or((l, ""));
            let inner = key.strip_prefix("credential.")?;
            let scope = if inner == "helper" {
                None
            } else {
                Some(inner.strip_suffix(".helper")?.to_string())
            };
            Some(HelperEntry {
                scope,
                value: val.to_string(),
            })
        })
        .collect()
}

/// URL-scoped global helpers that come AFTER ours and therefore override it for that host.
/// (Ones before ours are captured and delegated to, so they keep working.)
pub fn url_scoped_helpers() -> Vec<String> {
    let all = entries("--global");
    let Some(i) = all
        .iter()
        .position(|e| e.scope.is_none() && is_ours(&e.value))
    else {
        return vec![];
    };
    all[i + 1..]
        .iter()
        .filter_map(|e| e.scope.as_ref().map(|s| format!("credential.{s}.helper")))
        .collect()
}

/// Make plain `git push` ask git-vault first. git-vault becomes the ONLY helper (after a reset),
/// and delegates to the helpers it replaced when no vault applies. That way other helpers
/// (osxkeychain, `store`, ...) never see, and never `store`, a token that git-vault supplied.
/// Everything changed is recorded in config so `uninstall_helper` restores it exactly.
pub fn install_helper(cfg: &mut Config) -> Result<(String, Vec<String>)> {
    let exe = crate::util::stable_exe()?;
    let q = shlex::try_quote(&exe.to_string_lossy()).map(|c| c.into_owned())?;
    let ours = format!("!{q} credential");

    let global = all("--global");
    if cfg.helper.is_none() || !global.iter().any(|h| is_ours(h)) {
        let mut captured: Vec<HelperEntry> = entries("--system");
        captured.extend(entries("--global"));
        captured.retain(|e| !is_ours(&e.value));
        let mut fallbacks: Vec<String> = Vec::new();
        for h in all("--system").into_iter().chain(global.iter().cloned()) {
            if !h.is_empty() && !is_ours(&h) && !fallbacks.contains(&h) {
                fallbacks.push(h);
            }
        }
        let (had, val) = git_global(&["--get", "credential.useHttpPath"])?;
        cfg.helper = Some(HelperState {
            entries: captured,
            fallbacks,
            original_global: global,
            use_http_path: had.then(|| val.trim().to_string()),
        });
    }
    let _ = git_global(&["--unset-all", "credential.helper"]);
    for h in [String::new(), ours.clone()] {
        let (ok, _) = git_global(&["--add", "credential.helper", &h])?;
        if !ok {
            bail!("could not write credential.helper to global git config");
        }
    }
    git_global(&["credential.useHttpPath", "true"])?;
    Ok((ours, url_scoped_helpers()))
}

pub fn uninstall_helper(cfg: &mut Config) -> Result<()> {
    let _ = git_global(&["--unset-all", "credential.helper"]);
    match cfg.helper.take() {
        Some(st) => {
            for h in &st.original_global {
                if !is_ours(h) {
                    git_global(&["--add", "credential.helper", h])?;
                }
            }
            match st.use_http_path {
                Some(v) => {
                    git_global(&["credential.useHttpPath", &v])?;
                }
                None => {
                    let _ = git_global(&["--unset", "credential.useHttpPath"]);
                }
            }
        }
        None => {
            // no record (installed by an older version): only drop ours
            let _ = git_global(&["--unset", "credential.useHttpPath"]);
        }
    }
    Ok(())
}

pub fn helper_installed() -> bool {
    all("--global").iter().any(|h| is_ours(h))
}

/// Run one fallback helper the way git does.
fn helper_command(h: &str, op: &str) -> String {
    if let Some(cmd) = h.strip_prefix('!') {
        format!("{cmd} {op}")
    } else if std::path::Path::new(h).is_absolute() {
        let q = shlex::try_quote(h)
            .map(|c| c.into_owned())
            .unwrap_or_else(|_| h.to_string());
        format!("{q} {op}")
    } else {
        format!("git credential-{h} {op}")
    }
}

/// Does a `credential.<scope>.helper` section apply to this request?
fn scope_matches(scope: &str, proto: &str, host: &str, path: &str) -> bool {
    let Some((sp, rest)) = scope.split_once("://") else {
        return false;
    };
    let (shost, spath) = rest.split_once('/').unwrap_or((rest, ""));
    sp == proto
        && shost.eq_ignore_ascii_case(host)
        && (spath.is_empty()
            || path
                .trim_start_matches('/')
                .starts_with(spath.trim_end_matches('/')))
}

/// The helper list git would have used for this request had git-vault not been installed.
fn effective(st: &HelperState, proto: &str, host: &str, path: &str) -> Vec<String> {
    if st.entries.is_empty() {
        return st.fallbacks.clone(); // state written by 0.2.0
    }
    let mut out: Vec<String> = Vec::new();
    for e in &st.entries {
        if let Some(s) = &e.scope {
            if !scope_matches(s, proto, host, path) {
                continue;
            }
        }
        if e.value.is_empty() {
            out.clear();
        } else {
            out.push(e.value.clone());
        }
    }
    out
}

/// Pass a credential request through to the helpers git-vault replaced (scoped ones only for
/// the URLs they were scoped to). For `get`, the first helper that returns a password wins;
/// `store`/`erase` go to all of them.
pub fn delegate(st: &HelperState, op: &str, input: &str) -> Result<()> {
    let kv: std::collections::HashMap<&str, &str> =
        input.lines().filter_map(|l| l.split_once('=')).collect();
    let get = |k: &str| kv.get(k).copied().unwrap_or("");
    for h in effective(st, get("protocol"), get("host"), get("path")) {
        let mut c = Command::new("sh");
        c.arg("-c").arg(helper_command(&h, op));
        let Ok((ok, out, _)) = run_with_stdin(&mut c, input) else {
            continue;
        };
        if op == "get" && ok && out.contains("password=") {
            print!("{out}");
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(scope: Option<&str>, v: &str) -> HelperEntry {
        HelperEntry {
            scope: scope.map(str::to_string),
            value: v.to_string(),
        }
    }

    #[test]
    fn scoped_helpers_apply_only_to_their_url_and_resets_clear() {
        let st = HelperState {
            entries: vec![
                e(None, "osxkeychain"),
                e(Some("https://github.com"), ""),
                e(Some("https://github.com"), "!gh auth git-credential"),
            ],
            ..Default::default()
        };
        assert_eq!(
            effective(&st, "https", "github.com", "o/r"),
            vec!["!gh auth git-credential"]
        );
        assert_eq!(
            effective(&st, "https", "example.org", "o/r"),
            vec!["osxkeychain"]
        );
        assert_eq!(
            effective(&st, "http", "github.com", "o/r"),
            vec!["osxkeychain"]
        );
    }

    #[test]
    fn scope_with_path_prefix() {
        assert!(scope_matches(
            "https://github.com/corp",
            "https",
            "github.com",
            "corp/app.git"
        ));
        assert!(!scope_matches(
            "https://github.com/corp",
            "https",
            "github.com",
            "other/app.git"
        ));
    }
}
