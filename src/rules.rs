//! URL rules: map `host/owner/repo` patterns to vaults.
use crate::config::Config;

/// Rule target for a git credential request: `host[/path]`, lowercase, no port, no `.git`.
/// The path is taken literally, never parsed as a URL, and `..` disables rule matching.
pub fn request_target(host: &str, path: Option<&str>) -> Option<String> {
    let host = host.split(':').next()?.to_lowercase();
    if host.is_empty() {
        return None;
    }
    let mut segs = Vec::new();
    for s in path.unwrap_or("").split('/') {
        match s {
            "" | "." => {}
            ".." => return None,
            s => segs.push(s),
        }
    }
    let p = segs.join("/").to_lowercase();
    let p = p.strip_suffix(".git").unwrap_or(&p).to_string();
    Some(if p.is_empty() {
        host
    } else {
        format!("{host}/{p}")
    })
}

/// `*` matches any run of characters (including `/`).
pub fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn matches(pattern: &str, target: &str) -> bool {
    let pat = pattern.trim_end_matches('/').to_lowercase();
    glob(&pat, target) || (!pat.contains('*') && target.starts_with(&format!("{pat}/")))
}

/// First matching rule wins (config order).
pub fn pick<'a>(cfg: &'a Config, target: &str) -> Option<&'a str> {
    let t = target.to_lowercase();
    cfg.rules
        .iter()
        .find(|r| matches(&r.pattern, &t))
        .map(|r| r.vault.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Rule;

    #[test]
    fn request_target_is_literal() {
        assert_eq!(
            request_target("GitHub.com:443", Some("Corp/App.git")).unwrap(),
            "github.com/corp/app"
        );
        assert_eq!(
            request_target("git.corp.io:8443", Some("corp/app")).unwrap(),
            "git.corp.io/corp/app"
        );
        assert_eq!(request_target("github.com", None).unwrap(), "github.com");
        // URL/scp tricks in the path stay literal and cannot select another owner
        assert_eq!(
            request_target("github.com", Some("attacker/x@github.com:corp/app.git")).unwrap(),
            "github.com/attacker/x@github.com:corp/app"
        );
        assert!(request_target("github.com", Some("corp/../x")).is_none());
    }

    #[test]
    fn globs() {
        assert!(glob("github.com/corp/*", "github.com/corp/app"));
        assert!(glob("*/app", "github.com/corp/app"));
        assert!(!glob("github.com/corp/*", "github.com/other/app"));
        assert!(glob("*", "anything"));
    }

    #[test]
    fn first_rule_wins_and_prefix_matches() {
        let cfg = Config {
            rules: vec![
                Rule {
                    pattern: "github.com/corp/secret".into(),
                    vault: "special".into(),
                },
                Rule {
                    pattern: "github.com/corp".into(),
                    vault: "work".into(),
                },
                Rule {
                    pattern: "github.com".into(),
                    vault: "personal".into(),
                },
            ],
            ..Default::default()
        };
        assert_eq!(pick(&cfg, "github.com/corp/secret"), Some("special"));
        assert_eq!(pick(&cfg, "github.com/corp/app"), Some("work"));
        assert_eq!(pick(&cfg, "github.com/me/x"), Some("personal"));
        assert_eq!(pick(&cfg, "gitlab.com/x/y"), None);
    }
}
