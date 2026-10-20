#![allow(dead_code)]
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};

pub struct Req {
    pub method: String,
    pub path: String,
    pub headers: String,
    pub body: String,
}

/// Tiny HTTP server: handler maps a request to (status, json body). Requests are logged.
pub struct Mock {
    pub addr: String,
    pub log: Arc<Mutex<Vec<String>>>,
}

impl Mock {
    pub fn start(handler: impl Fn(&Req) -> (u16, String) + Send + 'static) -> Mock {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let log = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        std::thread::spawn(move || {
            for stream in l.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let head_end;
                loop {
                    let n = s.read(&mut tmp).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        head_end = p + 4;
                        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                        let cl = head
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        while buf.len() < head_end + cl {
                            let n = s.read(&mut tmp).unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&tmp[..n]);
                        }
                        let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                        let first = head.lines().next().unwrap_or("").to_string();
                        let mut it = first.split_whitespace();
                        let req = Req {
                            method: it.next().unwrap_or("").into(),
                            path: it.next().unwrap_or("").into(),
                            headers: head.to_lowercase(),
                            body,
                        };
                        log2.lock()
                            .unwrap()
                            .push(format!("{} {}", req.method, req.path));
                        let (code, body) = handler(&req);
                        let _ = write!(
                            s,
                            "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        break;
                    }
                }
            }
        });
        Mock { addr, log }
    }

    pub fn saw(&self, needle: &str) -> bool {
        self.log.lock().unwrap().iter().any(|l| l.contains(needle))
    }
}

pub struct Env {
    pub tmp: tempfile::TempDir,
}

impl Env {
    pub fn new() -> Env {
        let e = Env {
            tmp: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir_all(e.path("home")).unwrap();
        std::fs::create_dir_all(e.path("bin")).unwrap();
        e
    }

    pub fn path(&self, p: &str) -> PathBuf {
        self.tmp.path().join(p)
    }

    pub fn cmd(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut c = Command::new(program);
        let path = format!(
            "{}:{}",
            self.path("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        c.env("HOME", self.path("home"))
            .env("GIT_VAULT_HOME", self.path("vh"))
            .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_VAULT_BACKEND", "file")
            .env("GIT_VAULT_PASSPHRASE", "pw")
            .env("GIT_VAULT_SCRYPT_LOG_N", "8")
            .env("PATH", path)
            .env_remove("GIT_VAULT")
            .env_remove("GIT_VAULT_CONFIRM")
            .env_remove("GIT_VAULT_TOKEN")
            .env_remove("GIT_VAULT_CLIENT_ID")
            .env_remove("GIT_VAULT_SCHEME");
        c
    }

    pub fn vault(&self, args: &[&str]) -> Output {
        self.vault_in(None, args, "")
    }

    pub fn vault_in(&self, dir: Option<&Path>, args: &[&str], stdin: &str) -> Output {
        let mut c = self.cmd(env!("CARGO_BIN_EXE_git-vault"));
        c.args(args);
        if let Some(d) = dir {
            c.current_dir(d);
        }
        run(c, stdin)
    }

    /// Install an executable script into the test PATH.
    pub fn script(&self, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let p = self.path("bin").join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

pub fn run(mut c: Command, stdin: &str) -> Output {
    let mut child = c
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

pub fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}
pub fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}
#[track_caller]
pub fn ok(o: &Output) -> String {
    assert!(
        o.status.success(),
        "command failed\nstdout: {}\nstderr: {}",
        out(o),
        err(o)
    );
    out(o)
}
