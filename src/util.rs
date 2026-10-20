use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub const SERVICE: &str = "git-vault";

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Where config and (file-backend) secrets live. Override with GIT_VAULT_HOME.
pub fn data_dir() -> PathBuf {
    match std::env::var_os("GIT_VAULT_HOME") {
        Some(p) => PathBuf::from(p),
        None => home_dir().join(".config").join("git-vault"),
    }
}

pub fn expand_tilde(p: &str) -> String {
    if p == "~" {
        home_dir().to_string_lossy().into_owned()
    } else if let Some(rest) = p.strip_prefix("~/") {
        home_dir().join(rest).to_string_lossy().into_owned()
    } else {
        p.to_string()
    }
}

pub fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(bin))
        .find(|p| p.is_file())
}

/// Path to put in persisted git config: the PATH entry for git-vault when it is this very
/// binary (stable across Homebrew upgrades), else the real executable path.
pub fn stable_exe() -> Result<PathBuf> {
    let cur = std::env::current_exe().context("cannot locate git-vault binary")?;
    if let Some(p) = which("git-vault") {
        if let (Ok(a), Ok(b)) = (std::fs::canonicalize(&p), std::fs::canonicalize(&cur)) {
            if a == b {
                return Ok(p);
            }
        }
    }
    Ok(cur)
}

/// Always "https" in release builds. Debug builds honor GIT_VAULT_SCHEME so tests can talk to
/// local mock servers (a release binary must never be downgradable to plaintext via env).
pub fn scheme() -> String {
    if cfg!(debug_assertions) {
        std::env::var("GIT_VAULT_SCHEME").unwrap_or_else(|_| "https".into())
    } else {
        "https".into()
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Run a command feeding `stdin`; returns (success, stdout, stderr).
pub fn run_with_stdin(cmd: &mut Command, stdin: &str) -> Result<(bool, String, String)> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start {:?}", cmd.get_program()))?;
    child.stdin.take().unwrap().write_all(stdin.as_bytes())?;
    let out = child.wait_with_output()?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

pub fn run_capture(cmd: &mut Command) -> Result<(bool, String, String)> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to start {:?}", cmd.get_program()))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

/// Read a hidden line from the terminal (never from the git protocol on stdin).
pub fn prompt_secret(msg: &str) -> Result<String> {
    rpassword::prompt_password(msg).context("could not read from terminal")
}

/// Read a visible line from the terminal.
pub fn prompt_line(msg: &str) -> Result<String> {
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context("no terminal available for prompt")?;
    write!(tty, "{msg}")?;
    tty.flush()?;
    let mut line = String::new();
    BufReader::new(tty).read_line(&mut line)?;
    Ok(line.trim().to_string())
}

pub fn stdin_is_tty() -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .open("/dev/tty")
        .is_ok()
        && is_terminal_stdin()
}

fn is_terminal_stdin() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

pub fn git_global(args: &[&str]) -> Result<(bool, String)> {
    let mut c = Command::new("git");
    c.arg("config").arg("--global").args(args);
    let (ok, out, _) = run_capture(&mut c)?;
    Ok((ok, out))
}

pub fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base64() {
        assert_eq!(b64(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(b64(b"a"), "YQ==");
        assert_eq!(b64(b"ab"), "YWI=");
    }
}
