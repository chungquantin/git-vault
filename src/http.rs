use anyhow::{anyhow, Result};
use std::time::Duration;

pub enum Body<'a> {
    None,
    Form(&'a [(&'a str, &'a str)]),
    Json(serde_json::Value),
}

pub struct Resp {
    pub status: u16,
    pub body: String,
    /// Value of X-GitHub-SSO when the token still needs SAML authorization.
    pub sso: Option<String>,
}

impl Resp {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    pub fn json(&self) -> Result<serde_json::Value> {
        serde_json::from_str(&self.body).map_err(|e| {
            anyhow!(
                "bad JSON from server ({e}): {}",
                self.body.chars().take(160).collect::<String>()
            )
        })
    }
}

pub fn send(method: &str, url: &str, headers: &[(&str, &str)], body: Body) -> Result<Resp> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("git-vault/", env!("CARGO_PKG_VERSION")))
        .build();
    let mut req = agent.request(method, url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let res = match body {
        Body::None => req.call(),
        Body::Form(f) => req.send_form(f),
        Body::Json(v) => req.send_json(v),
    };
    let resp = match res {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(ureq::Error::Transport(t)) => return Err(anyhow!("{url}: {t}")),
    };
    let status = resp.status();
    let sso = resp.header("x-github-sso").map(str::to_string);
    let body = resp.into_string().unwrap_or_default();
    Ok(Resp { status, body, sso })
}
