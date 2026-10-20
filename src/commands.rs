use crate::cli::*;
use crate::config::{self, Config, Resolved, Rule, Vault};
use crate::confirm;
use crate::gitenv;
use crate::import;
use crate::provider::{self, Kind};
use crate::rules;
use crate::shell;
use crate::store::{self, Store};
use crate::util::*;
use anyhow::{anyhow, bail, Context, Result};
use clap::CommandFactory;
use std::collections::HashMap;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::Command;

pub struct Ctx {
    pub cfg: Config,
    pub backend: Option<String>,
}

impl Ctx {
    fn store(&self) -> Result<Box<dyn Store>> {
        store::open(self.backend.as_deref(), &self.cfg)
    }
    fn save(&self) -> Result<()> {
        config::save(&self.cfg)
    }
}

pub fn dispatch(cli: Cli) -> Result<()> {
    let mut ctx = Ctx {
        cfg: config::load()?,
        backend: cli.backend,
    };
    match cli.cmd {
        Cmd::Add(a) => add(&mut ctx, a),
        Cmd::Set(a) => set(&mut ctx, a),
        Cmd::Login(a) => login(&mut ctx, a),
        Cmd::Rotate { name } => rotate(&mut ctx, &name),
        Cmd::List { names } => list(&ctx, names),
        Cmd::Show { name } => show(&ctx, &name),
        Cmd::Remove { name, force } => remove(&mut ctx, &name, force),
        Cmd::Use { name } => use_vault(&ctx, &name),
        Cmd::Unuse => unuse(),
        Cmd::Rule { cmd } => rule(&mut ctx, cmd),
        Cmd::InstallHelper => install_helper(&mut ctx),
        Cmd::UninstallHelper => {
            shell::uninstall_helper(&mut ctx.cfg)?;
            ctx.save()?;
            println!("removed the git-vault credential helper and restored your previous global helper settings");
            Ok(())
        }
        Cmd::Run { args } => run(&ctx, args),
        Cmd::Exec { name, cmd } => exec(&ctx, &name, cmd),
        Cmd::Credential { vault, op } => credential(&ctx, vault, &op),
        Cmd::Import { what } => match what {
            ImportCmd::Gh { apply, force } => import_gh(&mut ctx, apply, force),
            ImportCmd::Ssh { apply, force, file } => import_ssh(&mut ctx, apply, force, file),
        },
        Cmd::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "git-vault",
                &mut std::io::stdout(),
            );
            Ok(())
        }
        Cmd::Doctor => doctor(&ctx),
        Cmd::ShellInit => {
            print!("{}", shell::SHELL_INIT);
            Ok(())
        }
    }
}

// ---------- vault CRUD ----------

fn apply_fields(v: &mut Vault, f: &Fields) -> Result<()> {
    fn put(slot: &mut Option<String>, val: &Option<String>) {
        if let Some(s) = val {
            *slot = if s.is_empty() { None } else { Some(s.clone()) };
        }
    }
    put(&mut v.user_name, &f.user_name);
    put(&mut v.email, &f.email);
    put(&mut v.host, &f.host);
    put(&mut v.provider, &f.provider);
    put(&mut v.username, &f.username);
    put(&mut v.api_url, &f.api_url);
    put(&mut v.client_id, &f.client_id);
    put(&mut v.signing_key, &f.signing_key);
    put(&mut v.signing_format, &f.signing_format);
    put(&mut v.confirm, &f.confirm);
    put(&mut v.extends, &f.extends);
    if let Some(k) = &f.ssh_key {
        if k.is_empty() {
            v.ssh_key = None;
        } else {
            let p = expand_tilde(k);
            if !std::path::Path::new(&p).exists() {
                bail!("ssh key not found: {k}");
            }
            v.ssh_key = Some(p);
        }
    }
    if f.sign.is_some() {
        v.sign = f.sign;
    }
    Ok(())
}

/// Resolve and sanity-check a vault after edits (extends target exists, no cycle, provider known).
fn validate(cfg: &Config, name: &str) -> Result<Resolved> {
    let r = cfg.resolve(name)?;
    Kind::detect(&r.v.host(), r.v.provider.as_deref())?;
    Ok(r)
}

fn read_token(prompt: &str, from_stdin: bool) -> Result<Option<String>> {
    if from_stdin {
        let mut s = String::new();
        std::io::stdin().read_line(&mut s)?;
        return Ok(Some(s.trim_end_matches(['\n', '\r']).to_string()).filter(|t| !t.is_empty()));
    }
    if let Ok(t) = std::env::var("GIT_VAULT_TOKEN") {
        return Ok(Some(t).filter(|t| !t.is_empty()));
    }
    if stdin_is_tty() {
        return Ok(Some(prompt_secret(prompt)?).filter(|t| !t.is_empty()));
    }
    Ok(None)
}

fn add(ctx: &mut Ctx, a: AddArgs) -> Result<()> {
    if !valid_name(&a.name) {
        bail!("vault name must match [A-Za-z0-9._-]+");
    }
    let exists = ctx.cfg.vaults.contains_key(&a.name);
    if exists && !a.force {
        bail!(
            "vault '{}' exists (use --force to overwrite metadata, or `set` to edit)",
            a.name
        );
    }
    let mut v = Vault::default();
    apply_fields(&mut v, &a.f)?;
    ctx.cfg.vaults.insert(a.name.clone(), v);
    let r = validate(&ctx.cfg, &a.name)?; // config is only saved on success

    let mut stored = false;
    if !a.no_token {
        let prompt = format!("Token/password for {} (empty to skip): ", r.v.host());
        if let Some(t) = read_token(&prompt, a.token_stdin)? {
            ctx.store()?.set(&a.name, &t)?;
            stored = true;
        }
    }
    ctx.save()?;
    println!(
        "added vault '{}'{}",
        a.name,
        if stored {
            " (token stored)"
        } else {
            " (no token)"
        }
    );
    if !stored && r.v.ssh_key.is_none() && r.v.extends.is_none() {
        println!("  next: git vault login --name {}", a.name);
    }
    Ok(())
}

fn set(ctx: &mut Ctx, a: SetArgs) -> Result<()> {
    let v = ctx
        .cfg
        .vaults
        .get_mut(&a.name)
        .ok_or_else(|| anyhow!("no such vault: {}", a.name))?;
    let before = v.clone();
    apply_fields(v, &a.f)?;
    if let Err(e) = validate(&ctx.cfg, &a.name) {
        ctx.cfg.vaults.insert(a.name.clone(), before);
        return Err(e);
    }
    ctx.save()?;
    println!("updated vault '{}' (token untouched)", a.name);
    Ok(())
}

fn list(ctx: &Ctx, names: bool) -> Result<()> {
    if ctx.cfg.vaults.is_empty() && !names {
        println!("no vaults. add one: git vault add --name <n> --user-name .. --email ..");
        return Ok(());
    }
    for n in ctx.cfg.vaults.keys() {
        if names {
            println!("{n}");
            continue;
        }
        let r = ctx.cfg.resolve(n)?;
        let kind = Kind::detect(&r.v.host(), r.v.provider.as_deref())
            .map(|k| k.name())
            .unwrap_or("?");
        let ext = r
            .chain
            .get(1)
            .map(|p| format!("extends {p}"))
            .unwrap_or_default();
        println!(
            "{n}\t{}\t{}\t{kind}\t{ext}",
            r.v.email.as_deref().unwrap_or("-"),
            r.v.host()
        );
    }
    Ok(())
}

fn show(ctx: &Ctx, name: &str) -> Result<()> {
    let r = ctx.cfg.resolve(name)?;
    let store = ctx.store()?;
    let tok = store::token_for(store.as_ref(), &ctx.cfg, &r)?;
    let own = &ctx.cfg.vaults[name];
    let out = serde_json::json!({
        "name": name,
        "backend": store.kind(),
        "own": own,
        "resolved": r.v,
        "token": { "present": tok.is_some(), "owner": tok.map(|t| t.0) },
        "rules": ctx.cfg.rules.iter().filter(|x| x.vault == name).map(|x| &x.pattern).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

fn remove(ctx: &mut Ctx, name: &str, force: bool) -> Result<()> {
    if !ctx.cfg.vaults.contains_key(name) {
        bail!("no such vault: {name}");
    }
    let deps = ctx.cfg.dependents(name);
    if !deps.is_empty() && !force {
        bail!(
            "vault(s) {} extend '{name}'; remove them first or use --force",
            deps.join(", ")
        );
    }
    ctx.cfg.vaults.remove(name);
    ctx.cfg.rules.retain(|r| r.vault != name);
    ctx.store()?.delete(name)?;
    ctx.save()?;
    println!("removed vault '{name}'");
    Ok(())
}

// ---------- login / rotate ----------

fn client_id_for(a: Option<&String>, r: &Resolved) -> Option<String> {
    a.cloned()
        .or_else(|| r.v.client_id.clone())
        .or_else(|| std::env::var("GIT_VAULT_CLIENT_ID").ok())
}

struct Acquired {
    token: String,
    username: Option<String>,
    token_name: Option<String>,
    client_id: Option<String>,
}

fn pick_method(a: &LoginArgs, kind: Kind, client_id: &Option<String>) -> String {
    if a.from_gh {
        return "gh-import".into();
    }
    match a.method.as_deref() {
        Some(m) if m != "auto" => m.to_string(),
        _ if client_id.is_some() && kind.supports_device_flow() => "device".into(),
        _ => match kind {
            Kind::GitHub => "gh",
            Kind::Gitea => "password",
            _ => "token",
        }
        .into(),
    }
}

fn acquire(kind: Kind, r: &Resolved, a: &LoginArgs) -> Result<Acquired> {
    let host = r.v.host();
    let cid = client_id_for(a.client_id.as_ref(), r);
    let method = pick_method(a, kind, &cid);
    let mut out = Acquired {
        token: String::new(),
        username: None,
        token_name: None,
        client_id: None,
    };
    match method.as_str() {
        "device" => {
            let cid = cid
                .ok_or_else(|| anyhow!("device flow needs --client-id (or GIT_VAULT_CLIENT_ID)"))?;
            let scope = a
                .scope
                .clone()
                .unwrap_or_else(|| kind.default_scope().to_string());
            out.token = provider::device_flow(kind, &host, &cid, &scope)?;
            out.client_id = Some(cid);
        }
        "gh" => {
            if kind != Kind::GitHub {
                bail!("--method gh only works for GitHub hosts");
            }
            eprintln!("Browser will open: sign in as the account for THIS vault (switch accounts on {host} first if needed).");
            out.token = provider::gh_web(&host)?;
        }
        "gh-import" => out.token = provider::gh_token(&host, a.gh_user.as_deref())?,
        "token" => {
            let hint = match kind {
                Kind::GitLab => "GitLab personal access token (read_user, write_repository)",
                Kind::Bitbucket => "Bitbucket app password",
                Kind::Gitea => "Gitea access token",
                Kind::GitHub => "GitHub token",
            };
            if kind == Kind::Bitbucket && r.v.username.is_none() {
                out.username = Some(prompt_line("Bitbucket username: ")?);
            }
            out.token = read_token(&format!("{hint}: "), a.token_stdin)?
                .ok_or_else(|| anyhow!("no token provided"))?;
        }
        "password" => {
            if kind != Kind::Gitea {
                bail!("--method password (mint a token with your password) is for Gitea/Forgejo");
            }
            let user = match &r.v.username {
                Some(u) => u.clone(),
                None => prompt_line("Gitea username: ")?,
            };
            let pw = match std::env::var("GIT_VAULT_PASSWORD") {
                Ok(p) => p,
                Err(_) => prompt_secret(&format!("Password for {user}@{host}: "))?,
            };
            let name = format!("git-vault-{}-{}", r.name, now_secs());
            let api = provider::api_base(kind, &host, r.v.api_url.as_deref());
            out.token = provider::gitea_mint(&api, &user, &pw, &name)?;
            out.username = Some(user);
            out.token_name = Some(name);
        }
        m => bail!("unknown method {m}"),
    }
    Ok(out)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn login(ctx: &mut Ctx, a: LoginArgs) -> Result<()> {
    let r = validate(&ctx.cfg, &a.name).map_err(|e| {
        anyhow!(
            "{e}. Create first: git vault add --name {} --user-name .. --email .. --no-token",
            a.name
        )
    })?;
    let kind = Kind::detect(&r.v.host(), r.v.provider.as_deref())?;
    let got = acquire(kind, &r, &a)?;
    let id = provider::whoami(
        kind,
        &r.v.host(),
        r.v.api_url.as_deref(),
        got.username.as_deref().or(r.v.username.as_deref()),
        &got.token,
    )
    .unwrap_or_else(|e| {
        eprintln!("warning: could not verify token: {e}");
        None
    });
    ctx.store()?.set(&a.name, &got.token)?;
    let v = ctx.cfg.vaults.get_mut(&a.name).unwrap();
    if got.username.is_some() {
        v.username = got.username;
    }
    v.token_name = got.token_name; // None clears a stale name from an earlier password login
    if got.client_id.is_some() && v.client_id.is_none() {
        v.client_id = got.client_id;
    }
    if let Some(i) = &id {
        if v.username.is_none() && matches!(kind, Kind::GitHub | Kind::Gitea | Kind::Bitbucket) {
            v.username = Some(i.login.clone());
        }
    }
    ctx.save()?;
    match id {
        Some(i) => println!("vault '{}' signed in as {}", a.name, i.login),
        None => println!("vault '{}' signed in (token not verified)", a.name),
    }
    Ok(())
}

/// The new token is already live on the host (the old one is gone): never lose it.
fn store_or_recover(store: &dyn Store, name: &str, token: &str) -> Result<()> {
    if let Err(e) = store.set(name, token) {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let p = data_dir().join(format!("recovery-{name}.token"));
        std::fs::create_dir_all(data_dir())?;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&p)?;
        f.write_all(token.as_bytes())?;
        bail!(
            "{e}\nThe NEW token is already active on the host, so it was saved to {} (mode 600).\nStore it with `git-vault login --name {name} --method token --token-stdin < {}`, then delete the file.",
            p.display(),
            p.display()
        );
    }
    Ok(())
}

fn rotate(ctx: &mut Ctx, name: &str) -> Result<()> {
    let r = validate(&ctx.cfg, name)?;
    let kind = Kind::detect(&r.v.host(), r.v.provider.as_deref())?;
    let store = ctx.store()?;
    let (owner, old) = store::token_for(store.as_ref(), &ctx.cfg, &r)?.ok_or_else(|| {
        anyhow!("vault '{name}' has no token; run `git vault login --name {name}`")
    })?;
    if owner != name {
        bail!("the token belongs to vault '{owner}'; rotate that one (`git-vault rotate --name {owner}`)");
    }
    let host = r.v.host();
    let api = provider::api_base(kind, &host, r.v.api_url.as_deref());
    let who =
        |t: &str, u: Option<&str>| provider::whoami(kind, &host, r.v.api_url.as_deref(), u, t);
    let old_id = who(&old, r.v.username.as_deref()).ok().flatten();
    // only this vault's own entry: `token_name` is never inherited
    let own_token_name = ctx.cfg.vaults[name].token_name.clone();

    match kind {
        Kind::GitLab => {
            let new = provider::gitlab_rotate(&api, &old)?;
            store_or_recover(store.as_ref(), name, &new)?; // GitLab already invalidated the old token
            let who_new = who(&new, None).ok().flatten();
            println!(
                "rotated vault '{name}': new token stored, old token revoked by GitLab{}",
                who_new
                    .map(|i| format!(" (as {})", i.login))
                    .unwrap_or_default()
            );
        }
        Kind::Gitea => {
            let user = r.v.username.clone().ok_or_else(|| {
                anyhow!("vault has no username; run `git-vault set --name {name} --username <you>`")
            })?;
            let pw = match std::env::var("GIT_VAULT_PASSWORD") {
                Ok(p) => p,
                Err(_) => prompt_secret(&format!("Password for {user}@{host}: "))?,
            };
            let new_name = format!("git-vault-{name}-{}", now_secs());
            let new = provider::gitea_mint(&api, &user, &pw, &new_name)?;
            if who(&new, None)?.is_none() {
                bail!("the new token does not authenticate; old token left in place");
            }
            store.set(name, &new)?;
            // record the new token's name before trying to revoke, so a failed revoke cannot desync it
            ctx.cfg.vaults.get_mut(name).unwrap().token_name = Some(new_name);
            ctx.save()?;
            match own_token_name {
                Some(old_name) => match provider::gitea_revoke(&api, &user, &pw, &old_name) {
                    Ok(true) => println!("rotated vault '{name}': new token stored, '{old_name}' revoked"),
                    Ok(false) => println!("rotated vault '{name}': new token stored. '{old_name}' was not found on {host}, so nothing was revoked; check your token list there"),
                    Err(e) => bail!("new token stored, but revoking '{old_name}' failed: {e}. Revoke it manually in {host} settings"),
                },
                None => println!("rotated vault '{name}': new token stored. The old token's name is unknown (it was not minted by git-vault); revoke it in {host} settings"),
            }
        }
        Kind::GitHub => {
            let la = LoginArgs {
                name: name.into(),
                method: None,
                client_id: None,
                scope: None,
                from_gh: false,
                gh_user: None,
                token_stdin: false,
            };
            let got = acquire(kind, &r, &la)?;
            let new_id = who(&got.token, None)?.ok_or_else(|| {
                anyhow!("the new token does not authenticate; old token left in place")
            })?;
            // Which account must the new token belong to? The old token's, else the vault's username.
            let expected = old_id
                .as_ref()
                .map(|i| i.login.clone())
                .or_else(|| r.v.username.clone());
            match expected {
                Some(e) if !e.eq_ignore_ascii_case(&new_id.login) => bail!(
                    "new token belongs to '{}' but this vault is '{e}'; sign in as the same account (old token left in place)",
                    new_id.login
                ),
                None => bail!(
                    "cannot verify which account this vault belongs to (the old token no longer works and no username is set). Run `git-vault set --name {name} --username {}` if that is right, then retry",
                    new_id.login
                ),
                _ => {}
            }
            store.set(name, &got.token)?;
            let cid = client_id_for(None, &r);
            match (cid, std::env::var("GIT_VAULT_CLIENT_SECRET")) {
                (Some(cid), Ok(secret)) => match provider::github_revoke(&api, &cid, &secret, &old) {
                    Ok(true) => println!("rotated vault '{name}' as {}: new token stored, old token revoked", new_id.login),
                    Ok(false) => println!(
                        "rotated vault '{name}' as {}: new token stored.\nThe old token was NOT revoked: GitHub does not know it for this OAuth app (it came from `gh` or a paste). Revoke it at https://{host}/settings/tokens",
                        new_id.login
                    ),
                    Err(e) => bail!("new token stored, but revoking the old one failed: {e}"),
                },
                _ => println!(
                    "rotated vault '{name}' as {}: new token stored.\nThe old token was NOT revoked (GitHub only allows that with the OAuth app's client id + GIT_VAULT_CLIENT_SECRET).\nRevoke it at https://{host}/settings/applications",
                    new_id.login
                ),
            }
        }
        Kind::Bitbucket => {
            let new = read_token("New Bitbucket app password: ", false)?
                .ok_or_else(|| anyhow!("no token provided"))?;
            if who(&new, r.v.username.as_deref())?.is_none() {
                bail!("the new app password does not authenticate; old one left in place");
            }
            store.set(name, &new)?;
            println!("rotated vault '{name}': new app password stored.\nBitbucket has no revoke API; delete the old one at https://bitbucket.org/account/settings/app-passwords/");
        }
    }
    Ok(())
}

// ---------- run / exec / credential ----------

fn split_vault(args: Vec<String>) -> Result<(Option<String>, Vec<String>)> {
    let (mut vault, mut rest) = (None, Vec::new());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--" {
            rest.push(a);
            rest.extend(it.by_ref());
            break;
        } else if a == "--vault" {
            vault = Some(it.next().ok_or_else(|| anyhow!("--vault needs a name"))?);
        } else if let Some(v) = a.strip_prefix("--vault=") {
            vault = Some(v.to_string());
        } else {
            rest.push(a);
        }
    }
    Ok((vault, rest))
}

fn run(ctx: &Ctx, args: Vec<String>) -> Result<()> {
    let (vault, rest) = split_vault(args)?;
    let vault = vault.or_else(|| std::env::var("GIT_VAULT").ok().filter(|v| !v.is_empty()));
    let mut cmd = Command::new("git");
    if let Some(v) = vault {
        let r = ctx.cfg.resolve(&v)?;
        let p = gitenv::prepare(&r)?;
        cmd.args(p.args).envs(p.env);
    }
    cmd.args(rest);
    Err(anyhow!("failed to run git: {}", cmd.exec()))
}

fn exec(ctx: &Ctx, name: &str, cmdline: Vec<String>) -> Result<()> {
    let r = ctx.cfg.resolve(name)?;
    let kind = Kind::detect(&r.v.host(), r.v.provider.as_deref())?;
    confirm::gate(&r.v.confirm_mode(), name, &format!("exec {}", cmdline[0]))?;
    let mut cmd = Command::new(&cmdline[0]);
    cmd.args(&cmdline[1..]).env("GIT_VAULT_ACTIVE", name);
    if let Some(n) = &r.v.user_name {
        cmd.env("GIT_AUTHOR_NAME", n).env("GIT_COMMITTER_NAME", n);
    }
    if let Some(e) = &r.v.email {
        cmd.env("GIT_AUTHOR_EMAIL", e).env("GIT_COMMITTER_EMAIL", e);
    }
    if let Some(s) = gitenv::ssh_command(&r) {
        cmd.env("GIT_SSH_COMMAND", s);
    }
    if let Some((_, tok)) = store::token_for(ctx.store()?.as_ref(), &ctx.cfg, &r)? {
        for k in kind.token_env() {
            cmd.env(k, &tok);
        }
        if kind == Kind::GitHub && r.v.host() != "github.com" {
            cmd.env("GH_HOST", r.v.host());
        }
    }
    Err(anyhow!("failed to run {}: {}", cmdline[0], cmd.exec()))
}

fn local_vault_name() -> Option<String> {
    let (ok, out, _) =
        run_capture(Command::new("git").args(["config", "--get", "vault.name"])).ok()?;
    ok.then(|| out.trim().to_string()).filter(|s| !s.is_empty())
}

/// Compare hosts exactly (port included), treating an explicit :443 as the https default.
fn same_host(a: &str, b: &str) -> bool {
    let n = |h: &str| h.to_lowercase().trim_end_matches(":443").to_string();
    n(a) == n(b)
}

/// The vault that should answer this credential request, or None. A vault only answers for
/// the exact host (and port) it is configured for, over https.
fn select_vault(ctx: &Ctx, explicit: Option<&str>, kv: &HashMap<&str, &str>) -> Option<Resolved> {
    if kv.get("protocol").is_some_and(|p| *p != "https") {
        return None;
    }
    let raw_host = kv.get("host").copied().unwrap_or("");
    let name = explicit
        .map(str::to_string)
        .or_else(local_vault_name)
        .or_else(|| {
            let target = rules::request_target(raw_host, kv.get("path").copied())?;
            rules::pick(&ctx.cfg, &target).map(str::to_string)
        })?;
    let r = match ctx.cfg.resolve(&name) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("git-vault: {e}");
            return None;
        }
    };
    same_host(&r.v.host(), raw_host).then_some(r)
}

/// git credential helper protocol. When no vault applies it stays silent (explicit --vault)
/// or hands the request to the helpers `install-helper` replaced, so they keep working for
/// every other host. Vault tokens are never passed to them.
fn credential(ctx: &Ctx, vault: Option<String>, op: &str) -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let kv: HashMap<&str, &str> = input.lines().filter_map(|l| l.split_once('=')).collect();

    let Some(r) = select_vault(ctx, vault.as_deref(), &kv) else {
        if vault.is_none() {
            if let Some(h) = &ctx.cfg.helper {
                shell::delegate(&h.fallbacks, op, &input)?;
            }
        }
        return Ok(());
    };
    if op != "get" {
        return Ok(()); // store/erase for a vault host are ignored: the token lives in the backend
    }
    let host = kv.get("host").copied().unwrap_or("");
    confirm::gate(&r.v.confirm_mode(), &r.name, host)?;
    let Some((_, token)) = store::token_for(ctx.store()?.as_ref(), &ctx.cfg, &r)? else {
        return Ok(());
    };
    let kind = Kind::detect(&r.v.host(), r.v.provider.as_deref())?;
    println!(
        "username={}",
        r.v.username.as_deref().unwrap_or(kind.default_username())
    );
    println!("password={token}");
    Ok(())
}

// ---------- use / rules / helper ----------

fn git_local(args: &[&str]) -> Result<bool> {
    let (ok, _, err) = run_capture(Command::new("git").arg("config").arg("--local").args(args))?;
    if !ok && !err.trim().is_empty() {
        bail!("git config failed: {}", err.trim());
    }
    Ok(ok)
}

fn use_vault(ctx: &Ctx, name: &str) -> Result<()> {
    let r = ctx.cfg.resolve(name)?;
    let (inside, _, _) = run_capture(Command::new("git").args(["rev-parse", "--git-dir"]))?;
    if !inside {
        bail!("not inside a git repository");
    }
    git_local(&["vault.name", name])?;
    for (k, v) in gitenv::identity_pairs(&r) {
        git_local(&[&k, &v])?;
    }
    if let Some(s) = gitenv::ssh_command(&r) {
        git_local(&["core.sshCommand", &s])?;
    }
    println!("this repo now uses vault '{name}' (identity, ssh key, signing, credentials)");
    if !shell::helper_installed() {
        println!("  for HTTPS tokens without --vault, also run: git vault install-helper");
    }
    Ok(())
}

fn unuse() -> Result<()> {
    for k in [
        "vault.name",
        "user.name",
        "user.email",
        "core.sshCommand",
        "gpg.format",
        "user.signingkey",
        "commit.gpgsign",
    ] {
        let _ = run_capture(Command::new("git").args(["config", "--local", "--unset-all", k]));
    }
    println!("removed vault settings from this repo's local git config");
    Ok(())
}

fn rule(ctx: &mut Ctx, cmd: RuleCmd) -> Result<()> {
    match cmd {
        RuleCmd::Add { pattern, vault } => {
            ctx.cfg.resolve(&vault)?;
            ctx.cfg.rules.retain(|r| r.pattern != pattern);
            ctx.cfg.rules.push(Rule {
                pattern: pattern.clone(),
                vault: vault.clone(),
            });
            ctx.save()?;
            println!("{pattern} -> {vault}");
        }
        RuleCmd::List => {
            if ctx.cfg.rules.is_empty() {
                println!("no rules. add one: git vault rule add 'github.com/corp/*' --vault work");
            }
            for r in &ctx.cfg.rules {
                println!("{}\t{}", r.pattern, r.vault);
            }
        }
        RuleCmd::Remove { pattern } => {
            let n = ctx.cfg.rules.len();
            ctx.cfg.rules.retain(|r| r.pattern != pattern);
            if ctx.cfg.rules.len() == n {
                bail!("no such rule: {pattern}");
            }
            ctx.save()?;
            println!("removed rule {pattern}");
        }
    }
    Ok(())
}

fn install_helper(ctx: &mut Ctx) -> Result<()> {
    let (h, scoped) = shell::install_helper(&mut ctx.cfg)?;
    ctx.save()?;
    println!("installed global credential helper: {h}");
    println!("  your previous helpers still answer for hosts no vault covers (git-vault delegates to them)");
    println!("  credential.useHttpPath=true is set so rules can match owner/repo");
    println!("  pick vaults with rules (git vault rule add 'github.com/corp/*' --vault work) or per repo (git vault use work)");
    if !scoped.is_empty() {
        println!("warning: URL-scoped helpers in your global git config can override git-vault for those hosts:");
        for k in scoped {
            println!("  {k}   (remove with: git config --global --unset-all {k})");
        }
    }
    Ok(())
}

// ---------- import ----------

fn import_gh(ctx: &mut Ctx, apply: bool, force: bool) -> Result<()> {
    let p = import::gh_hosts_path();
    let hosts = import::parse_gh_hosts(
        &import::read(&p).with_context(|| format!("reading {}", p.display()))?,
    );
    if hosts.is_empty() {
        println!("no gh accounts found in {}", p.display());
        return Ok(());
    }
    for (host, user) in hosts {
        let name = if host == "github.com" {
            user.clone()
        } else {
            format!("{user}-{}", host.split('.').next().unwrap_or(&host))
        };
        if ctx.cfg.vaults.contains_key(&name) && !force {
            println!("skip   {name} (exists; --force to overwrite)");
            continue;
        }
        if !apply {
            println!("would create vault '{name}' for {user}@{host}");
            continue;
        }
        let kind = match Kind::detect(&host, None) {
            Ok(k) => k,
            Err(e) => {
                println!("skip   {name}: {e}");
                continue;
            }
        };
        let token = match provider::gh_token(&host, Some(&user)) {
            Ok(t) => t,
            Err(e) => {
                println!("skip   {name}: {e}");
                continue;
            }
        };
        let id = provider::whoami(kind, &host, None, Some(&user), &token)
            .ok()
            .flatten();
        let email = id.as_ref().and_then(|i| i.email.clone()).or_else(|| {
            id.as_ref().and_then(|i| {
                i.id.map(|n| format!("{n}+{}@users.noreply.github.com", i.login))
            })
        });
        let v = Vault {
            user_name: id
                .as_ref()
                .and_then(|i| i.name.clone())
                .or(Some(user.clone())),
            email,
            host: Some(host.clone()),
            username: Some(user.clone()),
            ..Default::default()
        };
        ctx.store()?.set(&name, &token)?;
        ctx.cfg.vaults.insert(name.clone(), v);
        println!("created vault '{name}' ({user}@{host})");
    }
    if apply {
        ctx.save()?;
    } else {
        println!("dry run. re-run with --apply to create them");
    }
    Ok(())
}

fn import_ssh(ctx: &mut Ctx, apply: bool, force: bool, file: Option<String>) -> Result<()> {
    let p = file
        .map(|f| std::path::PathBuf::from(expand_tilde(&f)))
        .unwrap_or_else(import::ssh_config_path);
    let hosts = import::parse_ssh_config(
        &import::read(&p).with_context(|| format!("reading {}", p.display()))?,
    );
    if hosts.is_empty() {
        println!(
            "no Host blocks with HostName + IdentityFile in {}",
            p.display()
        );
        return Ok(());
    }
    for h in hosts {
        if !valid_name(&h.alias) {
            println!("skip   {} (not a valid vault name)", h.alias);
            continue;
        }
        if ctx.cfg.vaults.contains_key(&h.alias) && !force {
            println!("skip   {} (exists; --force to overwrite)", h.alias);
            continue;
        }
        if !apply {
            println!(
                "would create vault '{}' ({} with key {})",
                h.alias, h.hostname, h.identity
            );
            continue;
        }
        let provider = Kind::detect(&h.hostname, None)
            .ok()
            .map(|_| None)
            .unwrap_or(Some("github".to_string()));
        ctx.cfg.vaults.insert(
            h.alias.clone(),
            Vault {
                host: Some(h.hostname.clone()),
                ssh_key: Some(h.identity.clone()),
                provider,
                ..Default::default()
            },
        );
        println!(
            "created vault '{}'. next: git vault set --name {} --user-name .. --email ..",
            h.alias, h.alias
        );
    }
    if apply {
        ctx.save()?;
    } else {
        println!("dry run. re-run with --apply to create them");
    }
    Ok(())
}

// ---------- doctor ----------

fn doctor(ctx: &Ctx) -> Result<()> {
    let mut bad = 0;
    let mut row = |ok: bool, label: String, hint: &str| {
        if !ok {
            bad += 1;
        }
        println!(
            "[{}] {label}{}",
            if ok { "ok  " } else { "FAIL" },
            if ok || hint.is_empty() {
                String::new()
            } else {
                format!("  -> {hint}")
            }
        );
    };
    row(which("git").is_some(), "git".into(), "install git");
    row(
        which("git-vault").is_some(),
        format!("git-vault on PATH (v{})", env!("CARGO_PKG_VERSION")),
        "add its directory to PATH",
    );
    row(
        std::env::var("GIT_VAULT_SHELL").as_deref() == Ok("1"),
        "shell integration loaded".into(),
        "add: eval \"$(git-vault shell-init)\" to ~/.zshrc, then restart the shell",
    );
    let backend = ctx
        .backend
        .clone()
        .or_else(|| ctx.cfg.backend.clone())
        .unwrap_or_else(|| store::default_backend().to_string());
    let need = match backend.as_str() {
        "keychain" => Some("security"),
        "pass" => Some("pass"),
        "op" => Some("op"),
        "libsecret" => Some("secret-tool"),
        _ => None,
    };
    row(
        need.map(|b| which(b).is_some()).unwrap_or(true),
        format!("secret backend: {backend}"),
        "required tool not found on PATH",
    );
    println!(
        "[info] credential helper: {}",
        if shell::helper_installed() {
            "installed (plain `git push` uses rules)"
        } else {
            "not installed (git vault install-helper)"
        }
    );
    for k in shell::url_scoped_helpers() {
        println!("[warn] {k} in global git config can override git-vault for that host");
    }
    println!(
        "[info] gh {}",
        if which("gh").is_some() {
            "found"
        } else {
            "missing (needed for GitHub `login` without --client-id)"
        }
    );
    println!(
        "[info] {} vault(s), {} rule(s) in {}",
        ctx.cfg.vaults.len(),
        ctx.cfg.rules.len(),
        config::config_path().display()
    );
    if bad > 0 {
        std::process::exit(1);
    }
    Ok(())
}
