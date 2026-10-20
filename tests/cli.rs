mod common;
use common::*;
use std::path::Path;
use std::process::Command;

fn git(env: &Env, dir: &Path, args: &[&str]) -> String {
    let mut c = env.cmd("git");
    c.args(args).current_dir(dir);
    ok(&run(c, ""))
}

fn repo(env: &Env) -> std::path::PathBuf {
    let d = env.path("repo");
    std::fs::create_dir_all(&d).unwrap();
    git(env, &d, &["init", "-q", "-b", "main"]);
    d
}

fn add(env: &Env, name: &str, extra: &[&str], token: &str) {
    let mut a = vec!["add", "--name", name];
    a.extend_from_slice(extra);
    if token.is_empty() {
        a.push("--no-token");
    } else {
        a.push("--token-stdin");
    }
    ok(&env.vault_in(None, &a, &format!("{token}\n")));
}

fn cred(env: &Env, args: &[&str], req: &str) -> String {
    let mut a = vec!["credential"];
    a.extend_from_slice(args);
    a.push("get");
    out(&env.vault_in(None, &a, req))
}

#[test]
fn version_and_doctor() {
    let e = Env::new();
    assert!(out(&e.vault(&["--version"])).starts_with("git-vault "));
    let d = e.vault(&["doctor"]);
    assert!(out(&d).contains("secret backend: file"), "{}", out(&d));
}

#[test]
fn crud_and_inheritance() {
    let e = Env::new();
    add(
        &e,
        "work",
        &["--user-name", "W", "--email", "w@corp.io"],
        "tok-work",
    );
    add(
        &e,
        "work-ro",
        &["--extends", "work", "--email", "ro@corp.io"],
        "",
    );
    let shown = ok(&e.vault(&["show", "work-ro"]));
    let j: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(j["resolved"]["user_name"], "W"); // inherited
    assert_eq!(j["resolved"]["email"], "ro@corp.io"); // overridden
    assert_eq!(j["token"]["owner"], "work"); // token inherited
                                             // child can have its own scoped token
    ok(&e.vault_in(
        None,
        &[
            "login",
            "--name",
            "work-ro",
            "--method",
            "token",
            "--token-stdin",
        ],
        "tok-ro\n",
    ));
    let j: serde_json::Value = serde_json::from_str(&ok(&e.vault(&["show", "work-ro"]))).unwrap();
    assert_eq!(j["token"]["owner"], "work-ro");

    assert!(
        !e.vault(&["remove", "work"]).status.success(),
        "must refuse while work-ro extends it"
    );
    ok(&e.vault(&["remove", "work-ro"]));
    ok(&e.vault(&["remove", "work"]));
    assert!(out(&e.vault(&["list"])).contains("no vaults"));
}

#[test]
fn set_keeps_token_and_rejects_cycles() {
    let e = Env::new();
    add(&e, "a", &["--email", "a@x"], "tok-a");
    add(&e, "b", &["--extends", "a"], "");
    ok(&e.vault(&["set", "--name", "a", "--email", "new@x"]));
    let j: serde_json::Value = serde_json::from_str(&ok(&e.vault(&["show", "a"]))).unwrap();
    assert_eq!(j["token"]["present"], true);
    assert_eq!(j["resolved"]["email"], "new@x");
    assert!(
        !e.vault(&["set", "--name", "a", "--extends", "b"])
            .status
            .success(),
        "cycle"
    );
}

#[test]
fn run_injects_identity_and_signing() {
    let e = Env::new();
    add(
        &e,
        "v",
        &[
            "--user-name",
            "Vault User",
            "--email",
            "v@vault.dev",
            "--signing-key",
            "ABCDEF1234567890",
            "--sign",
            "false",
        ],
        "",
    );
    let r = repo(&e);
    ok(&e.vault_in(
        Some(&r),
        &[
            "run",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "x",
            "--vault",
            "v",
        ],
        "",
    ));
    assert_eq!(
        git(&e, &r, &["log", "-1", "--format=%an <%ae>"]).trim(),
        "Vault User <v@vault.dev>"
    );

    add(
        &e,
        "s",
        &[
            "--user-name",
            "S",
            "--email",
            "s@x",
            "--signing-key",
            "ABCDEF1234567890",
        ],
        "",
    );
    let fmt = ok(&e.vault_in(
        Some(&r),
        &["run", "--vault=s", "config", "--get", "gpg.format"],
        "",
    ));
    assert_eq!(fmt.trim(), "openpgp");
    let sign = ok(&e.vault_in(
        Some(&r),
        &["run", "config", "--get", "commit.gpgsign", "--vault", "s"],
        "",
    ));
    assert_eq!(sign.trim(), "true");

    // ssh key: GIT_SSH_COMMAND + ssh signing with the .pub
    let key = e.path("id_k");
    std::fs::write(&key, "k").unwrap();
    std::fs::write(e.path("id_k.pub"), "ssh-ed25519 AAAA").unwrap();
    add(&e, "k", &["--ssh-key", key.to_str().unwrap(), "--sign"], "");
    let sk = ok(&e.vault_in(
        Some(&r),
        &["run", "config", "--get", "user.signingkey", "--vault", "k"],
        "",
    ));
    assert!(sk.trim().ends_with("id_k.pub"), "{sk}");
    assert_eq!(
        ok(&e.vault_in(
            Some(&r),
            &["run", "config", "--get", "gpg.format", "--vault", "k"],
            ""
        ))
        .trim(),
        "ssh"
    );
}

#[test]
fn credential_helper_rules_host_match_and_use() {
    let e = Env::new();
    add(
        &e,
        "work",
        &[
            "--user-name",
            "W",
            "--email",
            "w@corp.io",
            "--username",
            "wuser",
        ],
        "tok-work",
    );
    add(
        &e,
        "me",
        &["--user-name", "Me", "--email", "me@x.io"],
        "tok-me",
    );
    ok(&e.vault(&["rule", "add", "github.com/corp/*", "--vault", "work"]));
    ok(&e.vault(&["rule", "add", "github.com", "--vault", "me"]));

    let req = |p: &str| format!("protocol=https\nhost=github.com\npath={p}\n\n");
    let w = cred(&e, &[], &req("corp/app.git"));
    assert!(
        w.contains("username=wuser") && w.contains("password=tok-work"),
        "{w}"
    );
    assert!(cred(&e, &[], &req("someone/else.git")).contains("password=tok-me"));
    // unknown host: stay silent so other helpers answer
    assert_eq!(
        cred(&e, &[], "protocol=https\nhost=gitlab.com\npath=x/y\n\n"),
        ""
    );
    // explicit --vault but wrong host: silent
    assert_eq!(
        cred(
            &e,
            &["--vault", "work"],
            "protocol=https\nhost=gitlab.com\n\n"
        ),
        ""
    );
    // store/erase are ignored
    assert_eq!(
        out(&e.vault_in(None, &["credential", "store"], "host=github.com\n\n")),
        ""
    );

    // `use` pins a repo and writes identity locally; helper then follows vault.name
    let r = repo(&e);
    ok(&e.vault_in(Some(&r), &["use", "work"], ""));
    assert_eq!(
        git(&e, &r, &["config", "--local", "user.email"]).trim(),
        "w@corp.io"
    );
    assert_eq!(
        git(&e, &r, &["config", "--local", "vault.name"]).trim(),
        "work"
    );
    let pinned = out(&e.vault_in(Some(&r), &["credential", "get"], &req("someone/else.git")));
    assert!(
        pinned.contains("password=tok-work"),
        "vault.name beats rules: {pinned}"
    );
    ok(&e.vault_in(Some(&r), &["unuse"], ""));
    assert!(!Command::new("git")
        .args(["config", "--local", "vault.name"])
        .current_dir(&r)
        .env("GIT_CONFIG_GLOBAL", e.path("gitconfig"))
        .output()
        .unwrap()
        .status
        .success());
}

#[test]
fn install_helper_replaces_then_delegates_and_restores_exactly() {
    let e = Env::new();
    // user's own setup: an empty reset (disables system helpers) then a helper, plus useHttpPath=false
    std::fs::write(
        e.path("gitconfig"),
        "[credential]\n\thelper =\n\thelper = fake\n\tuseHttpPath = false\n",
    )
    .unwrap();
    let log = e.path("fake.log");
    e.script(
        "git-credential-fake",
        &format!(r#"cat > /dev/null; echo "$1" >> "{}"; [ "$1" = get ] && printf 'username=fk\npassword=fallback\n'; exit 0"#, log.display()),
    );
    add(
        &e,
        "work",
        &["--email", "w@x", "--user-name", "W"],
        "tok-work",
    );
    ok(&e.vault(&["rule", "add", "github.com/corp/*", "--vault", "work"]));
    let home = e.path("home");
    let o = ok(&e.vault(&["install-helper"]));
    assert!(!o.contains("warning"), "{o}");

    // git-vault is the only helper (after a reset): other helpers can never `store` a vault token
    let list = git(
        &e,
        &home,
        &["config", "--global", "--get-all", "credential.helper"],
    );
    let lines: Vec<&str> = list.lines().collect();
    assert_eq!(lines.len(), 2, "{list:?}");
    assert_eq!(lines[0], "");
    assert!(
        lines[1].contains("git-vault") && lines[1].ends_with("credential"),
        "{list:?}"
    );
    assert_eq!(
        git(&e, &home, &["config", "--global", "credential.useHttpPath"]).trim(),
        "true"
    );
    ok(&e.vault(&["install-helper"])); // idempotent: still records the ORIGINAL state
    assert_eq!(
        git(
            &e,
            &home,
            &["config", "--global", "--get-all", "credential.helper"]
        ),
        list
    );

    // matched vault host: answered by git-vault, fallback never consulted, store ignored
    let req = "protocol=https\nhost=github.com\npath=corp/app.git\n\n";
    assert!(cred(&e, &[], req).contains("password=tok-work"));
    ok(&e.vault_in(
        None,
        &["credential", "store"],
        &format!("{req}username=git\npassword=tok-work\n"),
    ));
    assert!(!log.exists(), "vault token must never reach other helpers");
    // no vault applies: delegated to the replaced helper, for get AND store
    let other = "protocol=https\nhost=example.org\npath=a/b\n\n";
    assert!(cred(&e, &[], other).contains("password=fallback"));
    ok(&e.vault_in(None, &["credential", "store"], other));
    let seen = std::fs::read_to_string(&log).unwrap();
    assert!(seen.contains("get") && seen.contains("store"), "{seen}");
    // explicit --vault never delegates
    assert_eq!(cred(&e, &["--vault", "work"], other), "");

    ok(&e.vault(&["uninstall-helper"]));
    let after = git(
        &e,
        &home,
        &["config", "--global", "--get-all", "credential.helper"],
    );
    assert_eq!(after, "\nfake\n", "restored exactly, empty reset included");
    assert_eq!(
        git(&e, &home, &["config", "--global", "credential.useHttpPath"]).trim(),
        "false"
    );
}

#[test]
fn install_helper_warns_about_url_scoped_helpers() {
    let e = Env::new();
    std::fs::write(
        e.path("gitconfig"),
        "[credential \"https://github.com\"]\n\thelper =\n\thelper = !gh auth git-credential\n",
    )
    .unwrap();
    let o = ok(&e.vault(&["install-helper"]));
    assert!(
        o.contains("warning: URL-scoped helpers")
            && o.contains("credential.https://github.com.helper"),
        "{o}"
    );
    let d = out(&e.vault(&["doctor"]));
    assert!(
        d.contains("[warn] credential.https://github.com.helper"),
        "{d}"
    );
}

#[test]
fn inherited_token_never_crosses_hosts() {
    let e = Env::new();
    add(&e, "work", &["--email", "w@x"], "ghp_SECRET");
    add(
        &e,
        "work-gl",
        &["--extends", "work", "--host", "gitlab.com"],
        "",
    );
    assert_eq!(
        cred(
            &e,
            &["--vault", "work-gl"],
            "protocol=https\nhost=gitlab.com\n\n"
        ),
        ""
    );
    let j: serde_json::Value = serde_json::from_str(&ok(&e.vault(&["show", "work-gl"]))).unwrap();
    assert_eq!(j["token"]["present"], false);
    let o = ok(&e.vault(&["exec", "work-gl", "--", "sh", "-c", "echo [$GITLAB_TOKEN]"]));
    assert_eq!(o.trim(), "[]");
    // same host still inherits
    add(&e, "work-b", &["--extends", "work"], "");
    assert!(cred(
        &e,
        &["--vault", "work-b"],
        "protocol=https\nhost=github.com\n\n"
    )
    .contains("ghp_SECRET"));
}

#[test]
fn helper_requires_https_and_exact_host_and_port() {
    let e = Env::new();
    add(&e, "work", &["--email", "w@x"], "tok");
    assert!(cred(
        &e,
        &["--vault", "work"],
        "protocol=https\nhost=github.com\n\n"
    )
    .contains("password=tok"));
    assert!(cred(
        &e,
        &["--vault", "work"],
        "protocol=https\nhost=GitHub.com:443\n\n"
    )
    .contains("password=tok"));
    assert_eq!(
        cred(
            &e,
            &["--vault", "work"],
            "protocol=http\nhost=github.com\n\n"
        ),
        "",
        "plain http"
    );
    assert_eq!(
        cred(
            &e,
            &["--vault", "work"],
            "protocol=https\nhost=github.com:8080\n\n"
        ),
        "",
        "other port"
    );
    assert_eq!(
        cred(
            &e,
            &["--vault", "work"],
            "protocol=ssh\nhost=github.com\n\n"
        ),
        ""
    );
    // vault pinned to a port does not answer for the default port
    add(
        &e,
        "corp",
        &[
            "--host",
            "git.corp.io:8443",
            "--provider",
            "gitea",
            "--email",
            "c@x",
        ],
        "ctok",
    );
    assert!(cred(
        &e,
        &["--vault", "corp"],
        "protocol=https\nhost=git.corp.io:8443\n\n"
    )
    .contains("password=ctok"));
    assert_eq!(
        cred(
            &e,
            &["--vault", "corp"],
            "protocol=https\nhost=git.corp.io\n\n"
        ),
        ""
    );
    // rules still match a ported host (port is not part of the rule target)
    ok(&e.vault(&["rule", "add", "git.corp.io/corp/*", "--vault", "corp"]));
    let r = cred(
        &e,
        &[],
        "protocol=https\nhost=git.corp.io:8443\npath=corp/app.git\n\n",
    );
    assert!(r.contains("password=ctok"), "{r}");
}

#[test]
fn rules_cannot_be_steered_by_path_tricks() {
    let e = Env::new();
    add(&e, "work", &["--email", "w@x"], "tok-work");
    ok(&e.vault(&["rule", "add", "github.com/corp/*", "--vault", "work"]));
    for path in [
        "attacker/x@github.com:corp/app.git",
        "corp/../attacker/x",
        "attacker/../corp/app",
    ] {
        let r = cred(
            &e,
            &[],
            &format!("protocol=https\nhost=github.com\npath={path}\n\n"),
        );
        assert_eq!(r, "", "{path}");
    }
}

#[test]
fn invalid_vault_names_in_config_are_rejected() {
    let e = Env::new();
    std::fs::create_dir_all(e.path("vh")).unwrap();
    std::fs::write(
        e.path("vh/config.toml"),
        "[vaults.\"../x\"]\nemail = \"a@b\"\n",
    )
    .unwrap();
    let o = e.vault(&["list"]);
    assert!(!o.status.success());
    assert!(err(&o).contains("invalid vault name"), "{}", err(&o));
}

fn github_mock() -> Mock {
    let n = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let tok = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Mock::start(move |r| {
        match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/login/device/code") => (
            200,
            r#"{"device_code":"dc","user_code":"ABCD-1234","verification_uri":"http://x/device","interval":0,"expires_in":30}"#.into(),
        ),
        ("POST", "/login/oauth/access_token") => {
            if n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) % 2 == 0 {
                (200, r#"{"error":"authorization_pending"}"#.into())
            } else {
                let t = tok.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                (200, format!(r#"{{"access_token":"gho_tok{t}"}}"#))
            }
        }
        ("GET", "/api/v3/user") if r.headers.contains("bearer gho_tok") => {
            (200, r#"{"login":"mockuser","id":42,"name":"Mock User","email":null}"#.into())
        }
        ("DELETE", p) if p.starts_with("/api/v3/applications/") => (204, String::new()),
        _ => (404, r#"{"message":"nope"}"#.into()),
    }
    })
}

#[test]
fn github_device_login_then_rotate_with_revoke() {
    let m = github_mock();
    let e = Env::new();
    let host = m.addr.clone();
    let c = |args: &[&str]| {
        let mut cmd = e.cmd(env!("CARGO_BIN_EXE_git-vault"));
        cmd.env("GIT_VAULT_SCHEME", "http")
            .env("GIT_VAULT_CLIENT_SECRET", "sekret")
            .args(args);
        run(cmd, "")
    };
    ok(&c(&[
        "add",
        "--name",
        "g",
        "--host",
        &host,
        "--provider",
        "github",
        "--user-name",
        "M",
        "--email",
        "m@x",
        "--no-token",
    ]));
    let o = c(&["login", "--name", "g", "--client-id", "cid"]);
    assert!(
        out(&o).contains("signed in as mockuser"),
        "{}{}",
        out(&o),
        err(&o)
    );
    assert!(err(&o).contains("ABCD-1234"));

    let j: serde_json::Value = serde_json::from_str(&ok(&c(&["show", "g"]))).unwrap();
    assert_eq!(j["resolved"]["username"], "mockuser");
    assert_eq!(j["resolved"]["client_id"], "cid");

    let o = c(&["rotate", "--name", "g"]);
    assert!(
        out(&o).contains("old token revoked"),
        "{}{}",
        out(&o),
        err(&o)
    );
    assert!(m.saw("DELETE /api/v3/applications/cid/token"));
}

#[test]
fn github_rotate_without_secret_warns_not_revoked() {
    let m = github_mock();
    let e = Env::new();
    let host = m.addr.clone();
    let c = |args: &[&str]| {
        let mut cmd = e.cmd(env!("CARGO_BIN_EXE_git-vault"));
        cmd.env("GIT_VAULT_SCHEME", "http").args(args);
        run(cmd, "")
    };
    ok(&c(&[
        "add",
        "--name",
        "g",
        "--host",
        &host,
        "--provider",
        "github",
        "--client-id",
        "cid",
        "--no-token",
    ]));
    ok(&c(&["login", "--name", "g"]));
    let o = c(&["rotate", "--name", "g"]);
    assert!(out(&o).contains("NOT revoked"), "{}{}", out(&o), err(&o));
    assert!(!m.saw("DELETE"));
}

#[test]
fn gitlab_pat_rotate() {
    let m = Mock::start(|r| match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/api/v4/personal_access_tokens/self/rotate")
            if r.headers.contains("private-token: old-pat") =>
        {
            (
                200,
                r#"{"token":"new-pat","expires_at":"2027-01-01"}"#.into(),
            )
        }
        ("GET", "/api/v4/user") => (200, r#"{"username":"gl","id":1,"name":"GL"}"#.into()),
        _ => (401, "{}".into()),
    });
    let e = Env::new();
    let host = m.addr.clone();
    let c = |args: &[&str], stdin: &str| {
        let mut cmd = e.cmd(env!("CARGO_BIN_EXE_git-vault"));
        cmd.env("GIT_VAULT_SCHEME", "http").args(args);
        run(cmd, stdin)
    };
    ok(&c(
        &[
            "add",
            "--name",
            "gl",
            "--host",
            &host,
            "--provider",
            "gitlab",
            "--token-stdin",
        ],
        "old-pat\n",
    ));
    let o = c(&["rotate", "--name", "gl"], "");
    assert!(
        out(&o).contains("old token revoked by GitLab"),
        "{}{}",
        out(&o),
        err(&o)
    );
    let cr = c(&["credential", "get"], &format!("host={host}\n\n"));
    // no rule/use -> silent; ask explicitly
    assert_eq!(out(&cr), "");
    let cr = c(
        &["credential", "--vault", "gl", "get"],
        &format!("host={host}\n\n"),
    );
    assert!(
        out(&cr).contains("username=oauth2") && out(&cr).contains("password=new-pat"),
        "{}",
        out(&cr)
    );
}

#[test]
fn gitea_password_login_and_rotate_revokes_old_named_token() {
    let n = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let m = Mock::start(move |r| match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/api/v1/users/gu/tokens") if r.headers.contains("authorization: basic") => {
            let i = n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (201, format!(r#"{{"id":{i},"sha1":"gitea-tok{i}"}}"#))
        }
        ("DELETE", p) if p.starts_with("/api/v1/users/gu/tokens/git-vault-t-") => {
            (204, String::new())
        }
        ("GET", "/api/v1/user") if r.headers.contains("token gitea-tok") => {
            (200, r#"{"login":"gu","id":5,"full_name":"G U"}"#.into())
        }
        _ => (401, "{}".into()),
    });
    let e = Env::new();
    let host = m.addr.clone();
    let c = |args: &[&str]| {
        let mut cmd = e.cmd(env!("CARGO_BIN_EXE_git-vault"));
        cmd.env("GIT_VAULT_SCHEME", "http")
            .env("GIT_VAULT_PASSWORD", "pw123")
            .args(args);
        run(cmd, "")
    };
    ok(&c(&[
        "add",
        "--name",
        "t",
        "--host",
        &host,
        "--provider",
        "gitea",
        "--username",
        "gu",
        "--no-token",
    ]));
    let o = c(&["login", "--name", "t"]);
    assert!(
        out(&o).contains("signed in as gu"),
        "{}{}",
        out(&o),
        err(&o)
    );
    let o = c(&["rotate", "--name", "t"]);
    assert!(out(&o).contains("revoked"), "{}{}", out(&o), err(&o));
    assert!(m.saw("DELETE /api/v1/users/gu/tokens/git-vault-t-"));
    let cr = c(&["credential", "--vault", "t", "get"]);
    let _ = cr;
}

#[test]
fn failed_rotate_keeps_old_token() {
    // server rejects the new token -> must not overwrite the stored one
    let m = Mock::start(|r| match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/api/v1/users/gu/tokens") => (201, r#"{"sha1":"bad"}"#.into()),
        _ => (401, "{}".into()),
    });
    let e = Env::new();
    let host = m.addr.clone();
    let c = |args: &[&str], stdin: &str| {
        let mut cmd = e.cmd(env!("CARGO_BIN_EXE_git-vault"));
        cmd.env("GIT_VAULT_SCHEME", "http")
            .env("GIT_VAULT_PASSWORD", "x")
            .args(args);
        run(cmd, stdin)
    };
    ok(&c(
        &[
            "add",
            "--name",
            "t",
            "--host",
            &host,
            "--provider",
            "gitea",
            "--username",
            "gu",
            "--token-stdin",
        ],
        "keep-me\n",
    ));
    let o = c(&["rotate", "--name", "t"], "");
    assert!(!o.status.success());
    let cr = c(
        &["credential", "--vault", "t", "get"],
        &format!("host={host}\n\n"),
    );
    assert!(out(&cr).contains("password=keep-me"), "{}", out(&cr));
}

#[test]
fn fake_backends_pass_libsecret_op() {
    let e = Env::new();
    let store = e.path("fake-store");
    std::fs::create_dir_all(&store).unwrap();
    let s = store.display();
    e.script(
        "pass",
        &format!(
            r#"d="{s}"; case "$1" in
 show) f="$d/$(echo "$2" | tr / _)"; [ -f "$f" ] && cat "$f" || exit 1;;
 insert) cat > "$d/$(echo "$4" | tr / _)";;
 rm) rm -f "$d/$(echo "$3" | tr / _)";; esac"#
        ),
    );
    e.script(
        "secret-tool",
        &format!(
            r#"d="{s}"; case "$1" in
 lookup) f="$d/ss_$5"; [ -f "$f" ] && cat "$f" || exit 1;;
 store) cat > "$d/ss_$7";;
 clear) rm -f "$d/ss_$5";; esac"#
        ),
    );
    let opdb = store.join("op.json");
    let py = e.path("fake_op.py");
    std::fs::write(
        &py,
        format!(
            r#"import sys, json, os, uuid
db = "{db}"
items = json.load(open(db)) if os.path.exists(db) else []
a = sys.argv[1:]
def save(): json.dump(items, open(db, "w"))
if a[:2] == ["item", "list"]:
    print(json.dumps([{{"id": i["id"], "title": i["title"]}} for i in items]))
elif a[:2] == ["item", "get"]:
    m = [i for i in items if i["id"] == a[2]]
    if not m: sys.exit(1)
    print(m[0]["password"])
elif a[:2] == ["item", "create"]:
    t = json.load(open("/dev/stdin"))
    items.append({{"id": uuid.uuid4().hex[:8], "title": t["title"], "password": t["fields"][0]["value"]}}); save()
elif a[:2] == ["item", "delete"]:
    items[:] = [i for i in items if i["id"] != a[2]]; save()
else:
    sys.exit(2)
"#,
            db = opdb.display()
        ),
    )
    .unwrap();
    e.script("op", &format!("exec python3 {} \"$@\"", py.display()));

    for backend in ["pass", "libsecret", "op"] {
        let name = format!("v-{backend}");
        let o = e.vault_in(
            None,
            &[
                "--backend",
                backend,
                "add",
                "--name",
                &name,
                "--user-name",
                "U",
                "--email",
                "u@x",
                "--token-stdin",
            ],
            "secret-via-stdin\n",
        );
        ok(&o);
        assert!(out(&o).contains("token stored"));
        let cr = out(&e.vault_in(
            None,
            &["--backend", backend, "credential", "--vault", &name, "get"],
            "host=github.com\n\n",
        ));
        assert!(cr.contains("password=secret-via-stdin"), "{backend}: {cr}");
        if backend == "op" {
            // overwriting must replace, never duplicate (duplicates make `get` ambiguous)
            ok(&e.vault_in(
                None,
                &[
                    "--backend",
                    "op",
                    "login",
                    "--name",
                    &name,
                    "--method",
                    "token",
                    "--token-stdin",
                ],
                "second\n",
            ));
            let db: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&opdb).unwrap()).unwrap();
            assert_eq!(db.as_array().unwrap().len(), 1, "{db}");
            let cr = out(&e.vault_in(
                None,
                &["--backend", "op", "credential", "--vault", &name, "get"],
                "host=github.com\n\n",
            ));
            assert!(cr.contains("password=second"), "{cr}");
        }
        ok(&e.vault_in(None, &["--backend", backend, "remove", &name], ""));
        let cr = out(&e.vault_in(
            None,
            &["--backend", backend, "credential", "--vault", &name, "get"],
            "host=github.com\n\n",
        ));
        assert_eq!(cr, "", "{backend} after remove");
    }
}

#[test]
fn file_backend_is_encrypted_and_needs_right_passphrase() {
    let e = Env::new();
    add(&e, "v", &["--email", "a@b"], "plain-secret-123");
    let blob = std::fs::read(e.path("vh/secrets/v.age")).unwrap();
    assert!(!String::from_utf8_lossy(&blob).contains("plain-secret-123"));
    assert!(String::from_utf8_lossy(&blob).starts_with("age-encryption.org"));
    let mut c = e.cmd(env!("CARGO_BIN_EXE_git-vault"));
    c.env("GIT_VAULT_PASSPHRASE", "wrong")
        .args(["credential", "--vault", "v", "get"]);
    let o = run(c, "host=github.com\n\n");
    assert!(!o.status.success());
    assert!(err(&o).contains("wrong passphrase"), "{}", err(&o));
}

#[test]
fn import_gh_and_ssh() {
    let e = Env::new();
    let gh = e.path("gh");
    std::fs::create_dir_all(&gh).unwrap();
    std::fs::write(
        gh.join("hosts.yml"),
        "github.invalid:\n    users:\n        alice:\n        bob:\n    user: alice\n",
    )
    .unwrap();
    e.script(
        "gh",
        r#"[ "$1 $2" = "auth token" ] && echo "tok-$6" && exit 0; exit 1"#,
    );
    let run_import = |apply: bool| {
        let mut c = e.cmd(env!("CARGO_BIN_EXE_git-vault"));
        c.env("GH_CONFIG_DIR", &gh).args(["import", "gh"]);
        if apply {
            c.arg("--apply");
        }
        run(c, "")
    };
    let dry = ok(&run_import(false));
    assert!(
        dry.contains("would create vault 'alice-github'")
            || dry.contains("would create vault 'alice"),
        "{dry}"
    );
    assert!(
        out(&e.vault(&["list"])).contains("no vaults"),
        "dry run must not write"
    );
    let o = run_import(true);
    ok(&o);
    let list = out(&e.vault(&["list"]));
    assert!(list.contains("alice") && list.contains("bob"), "{list}");
    let cr = out(&e.vault_in(
        None,
        &["credential", "--vault", "alice-github", "get"],
        "host=github.invalid\n\n",
    ));
    assert!(cr.contains("password=tok-alice"), "{cr}");

    let cfg = e.path("ssh_config");
    std::fs::write(&cfg, "Host gh-work\n  HostName github.com\n  IdentityFile ~/.ssh/id_work\nHost *\n  ForwardAgent no\n").unwrap();
    let d = ok(&e.vault(&["import", "ssh", "--file", cfg.to_str().unwrap()]));
    assert!(d.contains("would create vault 'gh-work'"), "{d}");
    ok(&e.vault(&["import", "ssh", "--file", cfg.to_str().unwrap(), "--apply"]));
    let j: serde_json::Value = serde_json::from_str(&ok(&e.vault(&["show", "gh-work"]))).unwrap();
    assert!(
        j["own"]["ssh_key"]
            .as_str()
            .unwrap()
            .ends_with("/home/.ssh/id_work"),
        "{j}"
    );
}

#[test]
fn migrates_legacy_vaults_json() {
    let e = Env::new();
    std::fs::create_dir_all(e.path("vh")).unwrap();
    std::fs::write(
        e.path("vh/vaults.json"),
        r#"{"old":{"user_name":"Old","email":"o@x","host":"github.com","username":"ol","ssh_key":null,"signing_key":null}}"#,
    )
    .unwrap();
    let o = e.vault(&["list"]);
    assert!(out(&o).contains("old\to@x"), "{}{}", out(&o), err(&o));
    assert!(e.path("vh/config.toml").exists());
    assert!(e.path("vh/vaults.json.migrated").exists());
}

#[test]
fn completions_and_confirm_validation() {
    let e = Env::new();
    for sh in ["zsh", "bash", "fish"] {
        let o = ok(&e.vault(&["completions", sh]));
        assert!(o.contains("git-vault"), "{sh}");
    }
    add(&e, "v", &["--email", "a@b"], "t");
    let mut c = e.cmd(env!("CARGO_BIN_EXE_git-vault"));
    c.env("GIT_VAULT_CONFIRM", "bogus")
        .args(["credential", "--vault", "v", "get"]);
    let o = run(c, "host=github.com\n\n");
    assert!(!o.status.success());
    assert!(err(&o).contains("unknown confirm mode"), "{}", err(&o));
    assert!(!e
        .vault(&["add", "--name", "bad", "--confirm", "nope", "--no-token"])
        .status
        .success());
}

#[test]
fn exec_sets_identity_and_token_env() {
    let e = Env::new();
    add(
        &e,
        "v",
        &["--user-name", "X Y", "--email", "x@y.z"],
        "tok-exec",
    );
    let o = ok(&e.vault(&[
        "exec",
        "v",
        "--",
        "sh",
        "-c",
        "echo $GH_TOKEN $GIT_AUTHOR_EMAIL $GIT_VAULT_ACTIVE",
    ]));
    assert_eq!(o.trim(), "tok-exec x@y.z v");
}

#[test]
fn unknown_host_needs_provider() {
    let e = Env::new();
    let o = e.vault(&[
        "add",
        "--name",
        "x",
        "--host",
        "git.example.org",
        "--no-token",
    ]);
    assert!(!o.status.success());
    assert!(err(&o).contains("--provider"), "{}", err(&o));
    ok(&e.vault(&[
        "add",
        "--name",
        "x",
        "--host",
        "git.example.org",
        "--provider",
        "gitea",
        "--no-token",
    ]));
}

fn gitea_mock(delete_status: u16) -> Mock {
    let n = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Mock::start(move |r| match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/api/v1/users/gu/tokens") if r.headers.contains("authorization: basic") => {
            let i = n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (201, format!(r#"{{"sha1":"gitea-tok{i}"}}"#))
        }
        ("DELETE", p) if p.starts_with("/api/v1/users/gu/tokens/") => {
            (delete_status, String::new())
        }
        ("GET", "/api/v1/user")
            if r.headers.contains("token gitea-tok") || r.headers.contains("token pasted") =>
        {
            (200, r#"{"login":"gu","id":5}"#.into())
        }
        _ => (401, "{}".into()),
    })
}

fn with_http(e: &Env, args: &[&str], stdin: &str) -> std::process::Output {
    let mut cmd = e.cmd(env!("CARGO_BIN_EXE_git-vault"));
    cmd.env("GIT_VAULT_SCHEME", "http")
        .env("GIT_VAULT_PASSWORD", "pw")
        .args(args);
    run(cmd, stdin)
}

fn show_json(e: &Env, name: &str) -> serde_json::Value {
    serde_json::from_str(&ok(&e.vault(&["show", name]))).unwrap()
}

#[test]
fn gitlab_rotate_never_loses_the_new_token_when_storing_fails() {
    let m = Mock::start(|r| match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/api/v4/personal_access_tokens/self/rotate") => {
            (200, r#"{"token":"new-pat"}"#.into())
        }
        ("GET", "/api/v4/user") => (200, r#"{"username":"gl"}"#.into()),
        _ => (401, "{}".into()),
    });
    let e = Env::new();
    let store = e.path("fake-store");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("git-vault_gl"), "old-pat").unwrap();
    // pass that can read but not write
    e.script("pass", &format!(r#"case "$1" in show) cat "{}/$(echo "$2" | tr / _)";; insert) cat >/dev/null; exit 1;; esac"#, store.display()));
    let host = m.addr.clone();
    ok(&with_http(
        &e,
        &[
            "add",
            "--name",
            "gl",
            "--host",
            &host,
            "--provider",
            "gitlab",
            "--no-token",
        ],
        "",
    ));
    let o = with_http(&e, &["--backend", "pass", "rotate", "--name", "gl"], "");
    assert!(!o.status.success());
    assert!(
        err(&o).contains("NEW token is already active"),
        "{}",
        err(&o)
    );
    let rec = e.path("vh/recovery-gl.token");
    assert_eq!(std::fs::read_to_string(&rec).unwrap(), "new-pat");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&rec).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn gitea_rotate_on_child_never_revokes_the_parents_token() {
    let m = gitea_mock(204);
    let e = Env::new();
    let host = m.addr.clone();
    ok(&with_http(
        &e,
        &[
            "add",
            "--name",
            "p",
            "--host",
            &host,
            "--provider",
            "gitea",
            "--username",
            "gu",
            "--no-token",
        ],
        "",
    ));
    ok(&with_http(&e, &["login", "--name", "p"], "")); // mints, records token_name on the parent
    let parent_name = show_json(&e, "p")["own"]["token_name"]
        .as_str()
        .unwrap()
        .to_string();
    ok(&with_http(
        &e,
        &["add", "--name", "c", "--extends", "p", "--no-token"],
        "",
    ));
    ok(&with_http(
        &e,
        &["login", "--name", "c", "--method", "token", "--token-stdin"],
        "pasted\n",
    ));
    assert!(
        show_json(&e, "c")["resolved"]["token_name"].is_null(),
        "token_name must not be inherited"
    );

    let o = with_http(&e, &["rotate", "--name", "c"], "");
    ok(&o);
    assert!(
        !m.saw(&format!("DELETE /api/v1/users/gu/tokens/{parent_name}")),
        "revoked the parent's live token!"
    );
    assert!(
        !m.saw("DELETE"),
        "child's old token was pasted, its name is unknown: nothing to revoke"
    );
    assert!(
        out(&o).contains("old token's name is unknown"),
        "{}",
        out(&o)
    );
    assert_eq!(
        show_json(&e, "p")["own"]["token_name"],
        parent_name.as_str()
    );
}

#[test]
fn gitea_rotate_records_new_token_name_even_if_revoke_fails() {
    let m = gitea_mock(500);
    let e = Env::new();
    let host = m.addr.clone();
    ok(&with_http(
        &e,
        &[
            "add",
            "--name",
            "t",
            "--host",
            &host,
            "--provider",
            "gitea",
            "--username",
            "gu",
            "--no-token",
        ],
        "",
    ));
    ok(&with_http(&e, &["login", "--name", "t"], ""));
    let old = show_json(&e, "t")["own"]["token_name"]
        .as_str()
        .unwrap()
        .to_string();
    std::thread::sleep(std::time::Duration::from_millis(1100)); // names carry epoch seconds
    let o = with_http(&e, &["rotate", "--name", "t"], "");
    assert!(!o.status.success(), "revoke failed -> reported");
    assert!(err(&o).contains("Revoke it manually"), "{}", err(&o));
    let now = show_json(&e, "t")["own"]["token_name"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(
        now, old,
        "config must track the NEW token so the next rotate revokes the right one"
    );
}

#[test]
fn relogin_with_pasted_token_clears_stale_minted_name() {
    let m = gitea_mock(204);
    let e = Env::new();
    let host = m.addr.clone();
    ok(&with_http(
        &e,
        &[
            "add",
            "--name",
            "t",
            "--host",
            &host,
            "--provider",
            "gitea",
            "--username",
            "gu",
            "--no-token",
        ],
        "",
    ));
    ok(&with_http(&e, &["login", "--name", "t"], ""));
    assert!(show_json(&e, "t")["own"]["token_name"].is_string());
    ok(&with_http(
        &e,
        &["login", "--name", "t", "--method", "token", "--token-stdin"],
        "pasted\n",
    ));
    assert!(show_json(&e, "t")["own"]["token_name"].is_null());
}

#[test]
fn revoke_404_is_reported_as_not_revoked() {
    // GitHub: DELETE for a token this OAuth app does not know
    let n = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let m = Mock::start(move |r| {
        match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/login/device/code") => (200, r#"{"device_code":"d","user_code":"U","verification_uri":"x","interval":0,"expires_in":30}"#.into()),
        ("POST", "/login/oauth/access_token") => (200, format!(r#"{{"access_token":"gho_tok{}"}}"#, n.fetch_add(1, std::sync::atomic::Ordering::SeqCst))),
        ("GET", "/api/v3/user") => (200, r#"{"login":"mockuser","id":1}"#.into()),
        ("DELETE", _) => (404, "{}".into()),
        _ => (404, "{}".into()),
    }
    });
    let e = Env::new();
    let host = m.addr.clone();
    let c = |args: &[&str]| {
        let mut cmd = e.cmd(env!("CARGO_BIN_EXE_git-vault"));
        cmd.env("GIT_VAULT_SCHEME", "http")
            .env("GIT_VAULT_CLIENT_SECRET", "s")
            .args(args);
        run(cmd, "")
    };
    ok(&c(&[
        "add",
        "--name",
        "g",
        "--host",
        &host,
        "--provider",
        "github",
        "--client-id",
        "cid",
        "--no-token",
    ]));
    ok(&c(&["login", "--name", "g"]));
    let o = c(&["rotate", "--name", "g"]);
    assert!(
        out(&o).contains("NOT revoked") && !out(&o).contains("old token revoked"),
        "{}{}",
        out(&o),
        err(&o)
    );

    // Gitea: a stale token_name that no longer exists
    let gm = gitea_mock(404);
    let ghost = gm.addr.clone();
    ok(&with_http(
        &e,
        &[
            "add",
            "--name",
            "t",
            "--host",
            &ghost,
            "--provider",
            "gitea",
            "--username",
            "gu",
            "--no-token",
        ],
        "",
    ));
    ok(&with_http(&e, &["login", "--name", "t"], ""));
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let o = with_http(&e, &["rotate", "--name", "t"], "");
    assert!(
        out(&o).contains("nothing was revoked") && !out(&o).contains("' revoked"),
        "{}{}",
        out(&o),
        err(&o)
    );
}

#[test]
fn github_rotate_refuses_when_account_cannot_be_verified() {
    // old token is dead (whoami 401), vault has no username: any account would be accepted -> refuse
    let m = Mock::start(|r| {
        match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/login/device/code") => (200, r#"{"device_code":"d","user_code":"U","verification_uri":"x","interval":0,"expires_in":30}"#.into()),
        ("POST", "/login/oauth/access_token") => (200, r#"{"access_token":"gho_new"}"#.into()),
        ("GET", "/api/v3/user") if r.headers.contains("bearer gho_new") => (200, r#"{"login":"someone-else","id":9}"#.into()),
        _ => (401, "{}".into()),
    }
    });
    let e = Env::new();
    let host = m.addr.clone();
    let c = |args: &[&str], stdin: &str| {
        let mut cmd = e.cmd(env!("CARGO_BIN_EXE_git-vault"));
        cmd.env("GIT_VAULT_SCHEME", "http").args(args);
        run(cmd, stdin)
    };
    ok(&c(
        &[
            "add",
            "--name",
            "g",
            "--host",
            &host,
            "--provider",
            "github",
            "--client-id",
            "cid",
            "--token-stdin",
        ],
        "dead-token\n",
    ));
    let o = c(&["rotate", "--name", "g"], "");
    assert!(!o.status.success());
    assert!(
        err(&o).contains("cannot verify which account"),
        "{}",
        err(&o)
    );
    let cr = out(&e.vault_in(
        None,
        &["credential", "--vault", "g", "get"],
        &format!("host={host}\n\n"),
    ));
    // the old token is untouched
    let mut cmd = e.cmd(env!("CARGO_BIN_EXE_git-vault"));
    cmd.env("GIT_VAULT_SCHEME", "http")
        .args(["credential", "--vault", "g", "get"]);
    let cr2 = out(&run(cmd, &format!("host={host}\n\n")));
    let _ = cr;
    assert!(cr2.contains("password=dead-token"), "{cr2}");
}
