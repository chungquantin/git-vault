//! Git hosts: identity lookup, sign-in flows and token rotation.
use crate::http::{self, Body};
use crate::util::{b64, run_capture, scheme, stdin_is_tty, which};
use anyhow::{anyhow, bail, Context, Result};
use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    GitHub,
    GitLab,
    Gitea,
    Bitbucket,
}

#[derive(Debug, Clone)]
pub struct Identity {
    pub login: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub id: Option<u64>,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::GitHub => "github",
            Kind::GitLab => "gitlab",
            Kind::Gitea => "gitea",
            Kind::Bitbucket => "bitbucket",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s.to_ascii_lowercase().as_str() {
            "github" => Some(Kind::GitHub),
            "gitlab" => Some(Kind::GitLab),
            "gitea" | "forgejo" => Some(Kind::Gitea),
            "bitbucket" => Some(Kind::Bitbucket),
            _ => None,
        }
    }

    pub fn detect(host: &str, explicit: Option<&str>) -> Result<Kind> {
        if let Some(e) = explicit {
            return Kind::parse(e)
                .ok_or_else(|| anyhow!("unknown provider '{e}' (github|gitlab|gitea|bitbucket)"));
        }
        let h = host.split(':').next().unwrap_or(host);
        Ok(match h {
            "github.com" => Kind::GitHub,
            "gitlab.com" => Kind::GitLab,
            "bitbucket.org" => Kind::Bitbucket,
            "codeberg.org" => Kind::Gitea,
            _ if h.contains("gitlab") => Kind::GitLab,
            _ if h.contains("gitea") || h.contains("forgejo") => Kind::Gitea,
            _ if h.contains("github") => Kind::GitHub,
            _ => bail!("can't tell the provider for '{host}': pass --provider github|gitlab|gitea|bitbucket"),
        })
    }

    /// HTTPS username git sends alongside the token.
    pub fn default_username(self) -> &'static str {
        match self {
            Kind::GitLab => "oauth2",
            Kind::Bitbucket => "x-token-auth",
            _ => "git",
        }
    }

    pub fn token_env(self) -> &'static [&'static str] {
        match self {
            Kind::GitHub => &["GH_TOKEN", "GITHUB_TOKEN"],
            Kind::GitLab => &["GITLAB_TOKEN"],
            Kind::Gitea => &["GITEA_TOKEN"],
            Kind::Bitbucket => &["BITBUCKET_TOKEN"],
        }
    }

    pub fn supports_device_flow(self) -> bool {
        matches!(self, Kind::GitHub | Kind::GitLab)
    }

    pub fn default_scope(self) -> &'static str {
        match self {
            Kind::GitLab => "read_user write_repository",
            _ => "repo",
        }
    }
}

pub fn web_base(host: &str) -> String {
    format!("{}://{host}", scheme())
}

pub fn api_base(kind: Kind, host: &str, api_url: Option<&str>) -> String {
    if let Some(u) = api_url {
        return u.trim_end_matches('/').to_string();
    }
    let s = scheme();
    match kind {
        Kind::GitHub if host == "github.com" => "https://api.github.com".into(),
        Kind::GitHub => format!("{s}://{host}/api/v3"),
        Kind::GitLab => format!("{s}://{host}/api/v4"),
        Kind::Gitea => format!("{s}://{host}/api/v1"),
        Kind::Bitbucket => "https://api.bitbucket.org/2.0".into(),
    }
}

/// Who owns this token? Ok(None) when the host rejects it (warns about SSO).
pub fn whoami(
    kind: Kind,
    host: &str,
    api_url: Option<&str>,
    username: Option<&str>,
    token: &str,
) -> Result<Option<Identity>> {
    let api = api_base(kind, host, api_url);
    let auth = match kind {
        Kind::Gitea => format!("token {token}"),
        Kind::Bitbucket if username.is_some() => format!(
            "Basic {}",
            b64(format!("{}:{token}", username.unwrap()).as_bytes())
        ),
        _ => format!("Bearer {token}"),
    };
    let r = http::send(
        "GET",
        &format!("{api}/user"),
        &[("Authorization", &auth), ("Accept", "application/json")],
        Body::None,
    )?;
    if let Some(sso) = &r.sso {
        eprintln!("warning: token needs SSO authorization: {sso}");
    }
    if !r.ok() {
        return Ok(None);
    }
    let j = r.json()?;
    let s = |k: &str| j[k].as_str().map(str::to_string);
    let Some(login) = s("login")
        .or_else(|| s("username"))
        .or_else(|| s("nickname"))
    else {
        return Ok(None);
    };
    Ok(Some(Identity {
        login,
        name: s("name")
            .or_else(|| s("full_name"))
            .or_else(|| s("display_name"))
            .filter(|n| !n.is_empty()),
        email: s("email").filter(|e| !e.is_empty()),
        id: j["id"].as_u64(),
    }))
}

fn open_browser(url: &str) {
    if !stdin_is_tty() {
        return;
    }
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if which(opener).is_some() {
        let _ = Command::new(opener).arg(url).output();
    }
}

/// OAuth device authorization grant (GitHub, GitLab). You sign in (SSO included) in the browser.
pub fn device_flow(kind: Kind, host: &str, client_id: &str, scope: &str) -> Result<String> {
    let web = web_base(host);
    let (code_ep, token_ep) = match kind {
        Kind::GitHub => ("/login/device/code", "/login/oauth/access_token"),
        Kind::GitLab => ("/oauth/authorize_device", "/oauth/token"),
        k => bail!(
            "device flow is not available for {}; use --method token",
            k.name()
        ),
    };
    let r = http::send(
        "POST",
        &format!("{web}{code_ep}"),
        &[("Accept", "application/json")],
        Body::Form(&[("client_id", client_id), ("scope", scope)]),
    )?;
    let d = r.json()?;
    let Some(device_code) = d["device_code"].as_str() else {
        bail!(
            "device flow refused: {}",
            d["error_description"]
                .as_str()
                .or(d["error"].as_str())
                .unwrap_or("unknown error")
        );
    };
    let uri = d["verification_uri"].as_str().unwrap_or("");
    eprintln!(
        "Open {uri} and enter code: {}",
        d["user_code"].as_str().unwrap_or("?")
    );
    eprintln!("Complete your SSO sign-in there; waiting...");
    open_browser(uri);

    let mut interval = d["interval"].as_u64().unwrap_or(5);
    let deadline =
        Instant::now() + Duration::from_secs(d["expires_in"].as_u64().unwrap_or(900).min(3600));
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_secs(interval));
        let r = http::send(
            "POST",
            &format!("{web}{token_ep}"),
            &[("Accept", "application/json")],
            Body::Form(&[
                ("client_id", client_id),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ]),
        )?;
        let j = r.json()?;
        if let Some(t) = j["access_token"].as_str() {
            return Ok(t.to_string());
        }
        match j["error"].as_str() {
            Some("authorization_pending") => {}
            Some("slow_down") => interval = j["interval"].as_u64().unwrap_or(interval + 5),
            e => bail!(
                "login failed: {}",
                j["error_description"]
                    .as_str()
                    .or(e)
                    .unwrap_or("unknown error")
            ),
        }
    }
    bail!("login timed out")
}

/// Fresh GitHub browser sign-in via `gh` in a throwaway config dir (gh's own accounts stay untouched).
pub fn gh_web(host: &str) -> Result<String> {
    if which("gh").is_none() {
        bail!("`gh` not found: install it (brew install gh), or use --client-id for device flow");
    }
    use std::os::unix::fs::DirBuilderExt;
    let dir = std::env::temp_dir().join(format!(
        "git-vault-gh-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    // exclusive create, owner-only: fails if someone pre-created the path
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .context("could not create a private temp dir")?;
    let result = (|| -> Result<String> {
        let st = Command::new("gh")
            .args([
                "auth",
                "login",
                "--hostname",
                host,
                "--web",
                "--git-protocol",
                "https",
                "--skip-ssh-key",
            ])
            .env("GH_CONFIG_DIR", &dir)
            .env_remove("GH_TOKEN")
            .env_remove("GITHUB_TOKEN")
            .status()?;
        if !st.success() {
            bail!("gh sign-in failed");
        }
        let (ok, out, _) = run_capture(
            Command::new("gh")
                .args(["auth", "token", "--hostname", host])
                .env("GH_CONFIG_DIR", &dir),
        )?;
        if !ok || out.trim().is_empty() {
            bail!("could not read token after sign-in");
        }
        Ok(out.trim().to_string())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Token for an account `gh` already knows (`gh auth token`).
pub fn gh_token(host: &str, user: Option<&str>) -> Result<String> {
    if which("gh").is_none() {
        bail!("`gh` not found");
    }
    let mut c = Command::new("gh");
    c.args(["auth", "token", "--hostname", host]);
    if let Some(u) = user {
        c.args(["--user", u]);
    }
    let (ok, out, _) = run_capture(&mut c)?;
    if !ok || out.trim().is_empty() {
        bail!(
            "gh has no token for {host}{}. Run `gh auth login --hostname {host} --web` first",
            user.map(|u| format!(" / {u}")).unwrap_or_default()
        );
    }
    Ok(out.trim().to_string())
}

fn basic(user: &str, pass: &str) -> String {
    format!("Basic {}", b64(format!("{user}:{pass}").as_bytes()))
}

/// Gitea/Forgejo: mint a personal access token with basic auth. Returns the token.
pub fn gitea_mint(api: &str, user: &str, password: &str, name: &str) -> Result<String> {
    let r = http::send(
        "POST",
        &format!("{api}/users/{user}/tokens"),
        &[
            ("Authorization", &basic(user, password)),
            ("Accept", "application/json"),
        ],
        Body::Json(
            serde_json::json!({ "name": name, "scopes": ["write:repository", "read:repository", "read:user"] }),
        ),
    )?;
    if r.status != 201 && r.status != 200 {
        bail!(
            "could not create token (HTTP {}): {}",
            r.status,
            r.body.chars().take(160).collect::<String>()
        );
    }
    let j = r.json()?;
    j["sha1"]
        .as_str()
        .or(j["token"].as_str())
        .map(str::to_string)
        .context("server returned no token")
}

/// Ok(true) revoked, Ok(false) no such token (nothing was revoked).
pub fn gitea_revoke(api: &str, user: &str, password: &str, name: &str) -> Result<bool> {
    let r = http::send(
        "DELETE",
        &format!("{api}/users/{user}/tokens/{name}"),
        &[("Authorization", &basic(user, password))],
        Body::None,
    )?;
    match r.status {
        200 | 204 => Ok(true),
        404 => Ok(false),
        c => bail!("revoke failed (HTTP {c})"),
    }
}

/// GitLab: rotate a personal access token in place. The old token dies, the new one is returned.
pub fn gitlab_rotate(api: &str, old: &str) -> Result<String> {
    let r = http::send(
        "POST",
        &format!("{api}/personal_access_tokens/self/rotate"),
        &[("PRIVATE-TOKEN", old), ("Accept", "application/json")],
        Body::None,
    )?;
    if !r.ok() {
        bail!(
            "GitLab refused to rotate (HTTP {}). Only personal access tokens can self-rotate; for an OAuth token run `git-vault login`",
            r.status
        );
    }
    r.json()?["token"]
        .as_str()
        .map(str::to_string)
        .context("server returned no token")
}

/// GitHub: revoke an OAuth app token. Needs the app's client id + secret.
/// Ok(false) = GitHub does not know the token for that app (e.g. it came from `gh` or a paste).
pub fn github_revoke(api: &str, client_id: &str, secret: &str, old: &str) -> Result<bool> {
    let r = http::send(
        "DELETE",
        &format!("{api}/applications/{client_id}/token"),
        &[
            ("Authorization", &basic(client_id, secret)),
            ("Accept", "application/json"),
        ],
        Body::Json(serde_json::json!({ "access_token": old })),
    )?;
    match r.status {
        204 => Ok(true),
        404 => Ok(false),
        c => bail!("revoke failed (HTTP {c})"),
    }
}
