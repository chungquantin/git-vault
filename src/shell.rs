use crate::config::{Config, HelperState};
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

/// `credential.<url>.helper` entries in global config. They are evaluated for that URL and
/// can override our host-independent helper, so the user needs to know.
pub fn url_scoped_helpers() -> Vec<String> {
    let (_, out) = git_global(&["--get-regexp", r"^credential\..+\.helper$"])
        .unwrap_or((false, String::new()));
    out.lines()
        .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
        .filter(|k| !k.is_empty())
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
        let mut fallbacks: Vec<String> = Vec::new();
        for h in all("--system").into_iter().chain(global.iter().cloned()) {
            if !h.is_empty() && !is_ours(&h) && !fallbacks.contains(&h) {
                fallbacks.push(h);
            }
        }
        let (had, val) = git_global(&["--get", "credential.useHttpPath"])?;
        cfg.helper = Some(HelperState {
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

/// Pass a credential request through to the helpers git-vault replaced. For `get`, the first
/// helper that returns a password wins; `store`/`erase` go to all of them.
pub fn delegate(fallbacks: &[String], op: &str, input: &str) -> Result<()> {
    for h in fallbacks {
        let mut c = Command::new("sh");
        c.arg("-c").arg(helper_command(h, op));
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
