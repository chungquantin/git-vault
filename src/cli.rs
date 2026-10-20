use clap::{Args, Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser)]
#[command(
    name = "git-vault",
    version,
    about = "Multiple git identities and credentials, picked per command (aws-vault for git)",
    propagate_version = true
)]
pub struct Cli {
    /// Secret backend: keychain | file | pass | op | libsecret
    #[arg(long, global = true, env = "GIT_VAULT_BACKEND", value_parser = ["keychain", "file", "pass", "op", "libsecret"])]
    pub backend: Option<String>,
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// Vault fields shared by `add` and `set`. Pass "" to clear a field with `set`.
#[derive(Args, Default, Clone)]
pub struct Fields {
    /// user.name for commits
    #[arg(long)]
    pub user_name: Option<String>,
    /// user.email for commits
    #[arg(long)]
    pub email: Option<String>,
    /// Git host (default github.com)
    #[arg(long)]
    pub host: Option<String>,
    /// github | gitlab | gitea | bitbucket (auto-detected for well-known hosts)
    #[arg(long)]
    pub provider: Option<String>,
    /// HTTPS username (default depends on provider)
    #[arg(long)]
    pub username: Option<String>,
    /// API base URL for self-hosted instances
    #[arg(long)]
    pub api_url: Option<String>,
    /// OAuth app client id for device-flow login
    #[arg(long)]
    pub client_id: Option<String>,
    /// SSH private key for ssh remotes (also used to sign if --sign)
    #[arg(long)]
    pub ssh_key: Option<String>,
    /// Signing key: path/.pub for ssh signing, or a GPG key id
    #[arg(long)]
    pub signing_key: Option<String>,
    /// ssh | openpgp (auto-detected)
    #[arg(long, value_parser = ["ssh", "openpgp"])]
    pub signing_format: Option<String>,
    /// Sign commits (uses --signing-key, else the ssh key's .pub)
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub sign: Option<bool>,
    /// Ask before releasing the token: none | terminal | dialog | touchid
    #[arg(long, value_parser = ["none", "terminal", "dialog", "touchid"])]
    pub confirm: Option<String>,
    /// Inherit every unset field (and the token) from another vault
    #[arg(long)]
    pub extends: Option<String>,
}

#[derive(Args)]
pub struct AddArgs {
    #[arg(long)]
    pub name: String,
    #[command(flatten)]
    pub f: Fields,
    /// Read the token from stdin
    #[arg(long)]
    pub token_stdin: bool,
    /// Do not prompt for a token (use `login` later)
    #[arg(long)]
    pub no_token: bool,
    /// Overwrite metadata of an existing vault (keeps its token)
    #[arg(long)]
    pub force: bool,
}

#[derive(Args)]
pub struct SetArgs {
    #[arg(long)]
    pub name: String,
    #[command(flatten)]
    pub f: Fields,
}

#[derive(Args)]
pub struct LoginArgs {
    #[arg(long)]
    pub name: String,
    /// auto | device | gh | gh-import | token | password
    #[arg(long, value_parser = ["auto", "device", "gh", "gh-import", "token", "password"])]
    pub method: Option<String>,
    /// OAuth client id for device flow (or GIT_VAULT_CLIENT_ID)
    #[arg(long)]
    pub client_id: Option<String>,
    /// OAuth scopes for device flow
    #[arg(long)]
    pub scope: Option<String>,
    /// Import the account gh already has (alias for --method gh-import)
    #[arg(long)]
    pub from_gh: bool,
    /// gh account to import when gh has several
    #[arg(long)]
    pub gh_user: Option<String>,
    /// Read a pasted token from stdin (--method token)
    #[arg(long)]
    pub token_stdin: bool,
}

#[derive(Subcommand)]
pub enum RuleCmd {
    /// Route URLs matching PATTERN (host/owner/repo, `*` wildcard) to a vault
    Add {
        pattern: String,
        #[arg(long)]
        vault: String,
    },
    List,
    Remove {
        pattern: String,
    },
}

#[derive(Subcommand)]
pub enum ImportCmd {
    /// Create vaults from accounts the gh CLI already knows
    Gh {
        /// Actually create them (default is a dry run)
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        force: bool,
    },
    /// Create vaults from Host blocks in ~/.ssh/config
    Ssh {
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        file: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Create a vault
    Add(AddArgs),
    /// Edit vault fields (token untouched)
    Set(SetArgs),
    /// Browser/SSO sign-in (GitHub, GitLab, Gitea, Bitbucket)
    Login(LoginArgs),
    /// Mint a new token, store it, revoke the old one
    Rotate {
        #[arg(long)]
        name: String,
    },
    /// List vaults
    List {
        /// Names only (for scripts and completion)
        #[arg(long)]
        names: bool,
    },
    /// Show a vault (never prints secrets)
    Show { name: String },
    /// Delete a vault and its token
    Remove {
        name: String,
        #[arg(long)]
        force: bool,
    },
    /// Pin this repo to a vault: writes identity + vault.name to local git config
    Use { name: String },
    /// Undo `use` for this repo
    Unuse,
    /// Manage URL rules used by the credential helper
    Rule {
        #[command(subcommand)]
        cmd: RuleCmd,
    },
    /// Make plain `git push` pick a vault by rule/`use`, no shell function needed
    InstallHelper,
    /// Remove the global credential helper
    UninstallHelper,
    /// Run git with a vault: git-vault run <git args> --vault NAME
    Run {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
        args: Vec<String>,
    },
    /// Run any command with the vault's identity and token in its environment
    Exec {
        name: String,
        #[arg(last = true, required = true)]
        cmd: Vec<String>,
    },
    /// git credential helper (internal)
    #[command(hide = true)]
    Credential {
        #[arg(long)]
        vault: Option<String>,
        #[arg(default_value = "get")]
        op: String,
    },
    /// Import vaults from gh accounts or ~/.ssh/config
    Import {
        #[command(subcommand)]
        what: ImportCmd,
    },
    /// Print a shell completion script
    Completions { shell: Shell },
    /// Check install, backends and shell integration
    Doctor,
    /// Print the shell function that enables `git add vault` and `--vault`
    ShellInit,
}
