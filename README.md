# git-vault

Multiple git identities and credentials, picked per command. Like `aws-vault`, for git. Written in Rust.

```bash
git commit --vault work -m "fix: typo"
git push   --vault personal
```

Author, HTTPS token, SSH key and signing key switch together, for that one command. Nothing is written to your git config (unless you opt in with `use` / `install-helper`).

## Install

```bash
# Homebrew (needs the tap, see "Releasing")
brew install chungquantin/tap/git-vault

# curl: prebuilt binary, checksum verified (falls back to `cargo install`)
curl -fsSL https://raw.githubusercontent.com/chungquantin/git-vault/main/install.sh | sh -s -- --shell

# from a clone
./install.sh            # cargo build --release + symlink
```

Enable the shell function (`--shell` above does this for you) and check the setup:

```bash
echo 'eval "$(git-vault shell-init)"' >> ~/.zshrc && source ~/.zshrc
git-vault doctor
```

`PREFIX=` changes the install dir (default `~/.local/bin`), `GIT_VAULT_VERSION=v0.2.0` pins a release, `install.sh --uninstall` removes it.

## Use

One vault per account. Each does its own sign-in:

```bash
git add vault --name personal --user-name "Me" --email me@gmail.com --no-token
git vault login --name personal        # browser sign-in

git add vault --name work --user-name "Me" --email me@corp.com --ssh-key ~/.ssh/id_work --no-token

git clone  --vault work git@github.com:corp/app.git
git commit --vault work -m "msg"
git push   --vault work
GIT_VAULT=work git pull                # env alternative
```

`git add vault` and `--vault` need the shell function from `shell-init` (git can't grow new flags). `git vault <cmd>` works without it.

```bash
git vault list | show NAME | remove NAME
git vault set --name work --email new@corp.com     # edit fields, token untouched
git-vault exec work -- gh pr create                # any tool, with GH_TOKEN + identity in its env
```

## No `--vault` needed: rules, `use`, global helper

```bash
git vault rule add 'github.com/corp/*' --vault work
git vault rule add 'github.com'        --vault personal      # first matching rule wins
git vault install-helper                                     # once: plain `git push` now picks a vault

cd ~/code/side-project && git vault use personal             # or pin a repo (identity + ssh key + signing)
```

`install-helper` makes git-vault the only global helper and *delegates* to the helpers it replaced (osxkeychain, gh, ...) for hosts no vault covers, so those helpers never see or `store` a vault token. `uninstall-helper` restores your previous settings exactly. URL-scoped helpers (`credential.https://github.com.helper`, e.g. from `gh auth setup-git`) can override git-vault for that host; `install-helper` and `doctor` warn about them.

Selection order for the credential helper: `--vault` / `GIT_VAULT` → the repo's `vault.name` (from `use`) → rules. If nothing matches, or the host differs from the vault's, git-vault stays silent so your other credential helpers keep working.

## Providers and sign-in

| Provider | `login` | `rotate` |
|---|---|---|
| GitHub / Enterprise | `gh` browser sign-in (default), or OAuth device flow with `--client-id` | re-login, then revoke the old token if `GIT_VAULT_CLIENT_SECRET` + client id are set |
| GitLab / self-hosted | device flow (`--client-id`) or paste a PAT | PAT self-rotation (old token dies) |
| Gitea / Forgejo / Codeberg | mint a token with your password (`--method password`) or paste one | mint new, store, delete the old one by name |
| Bitbucket | paste an app password | paste a new one (no revoke API) |

Unknown hosts need `--provider github|gitlab|gitea|bitbucket`. Other methods: `--method gh-import [--gh-user U]` takes an account `gh` already has; `--method token --token-stdin` for scripts. Rotation stores the new token only after it authenticates, so a failure never loses the working one.

```bash
git vault rotate --name work
git vault import gh --apply        # vaults from every account gh knows (dry run without --apply)
git vault import ssh --apply       # vaults from Host blocks in ~/.ssh/config
```

## Backends

`--backend` (or `GIT_VAULT_BACKEND`, or `backend =` in `config.toml`):

| Backend | Where tokens live |
|---|---|
| `keychain` | macOS Keychain (default on macOS) |
| `libsecret` | Linux Secret Service via `secret-tool` (default on Linux when present) |
| `pass` | passwordstore.org, entries `git-vault/<vault>` |
| `op` | 1Password CLI, items `git-vault <vault>` (`GIT_VAULT_OP_VAULT` picks the vault) |
| `file` | age (scrypt) encrypted files; passphrase from `GIT_VAULT_PASSPHRASE` or a prompt |

## Signing

```bash
git vault add --name work ... --ssh-key ~/.ssh/id_work --sign         # signs with id_work.pub (gpg.format=ssh)
git vault add --name gpg  ... --signing-key ABCDEF1234567890 --sign    # GPG key id
```

Add the same key on GitHub as a **Signing Key** for the "Verified" badge.

## Profile inheritance

```bash
git vault add --name work-readonly --extends work --email ro@corp.com --no-token
git vault login --name work-readonly --method token --token-stdin      # its own scoped token
```

Unset fields come from the parent, including the token if the child has none.

## Confirm before releasing a token

```bash
git vault set --name work --confirm dialog     # none | terminal | dialog | touchid
```

The prompt appears whenever the credential helper or `exec` is about to hand out the token (`GIT_VAULT_CONFIRM` overrides). `touchid` builds a tiny LocalAuthentication helper with `swiftc` on first use. It is a gate in git-vault, not Keychain access control: Keychain biometric ACLs require a code-signed binary with entitlements, which a CLI install is not.

## Config

`~/.config/git-vault/config.toml` (override the dir with `GIT_VAULT_HOME`); secrets are never in it.

```toml
backend = "keychain"

[vaults.work]
user_name = "Me"
email = "me@corp.com"
ssh_key = "/Users/me/.ssh/id_work"
sign = true

[vaults.work-readonly]
extends = "work"

[[rules]]
match = "github.com/corp/*"
vault = "work"
```

An older `vaults.json` (v0.1) is migrated automatically. Shell completions: `git-vault completions zsh|bash|fish`.

## Security notes

- A token is released only over `https`, to the vault's exact host and port. A vault that `extends` another inherits the parent's token only if the host/provider match.
- Rule matching uses the literal request path (no URL parsing, `..` disables rules).
- Rotation stores the new token before revoking the old one; if storing fails after a host already revoked the old token (GitLab), the new token is saved to `recovery-<vault>.token` (mode 600) instead of being lost.
- Secrets never go through argv (`security -i`, stdin for `pass`/`op`/`secret-tool`).
- Plain-HTTP hosts are not supported in release builds.

## Claude Code skill

`skills/git-vault/SKILL.md` teaches Claude Code to use vaults (raw `git-vault run ... --vault X`, vault selection, agent limits).

```bash
npx skills add chungquantin/git-vault -g
```

## How it works

For one git invocation only, `git-vault run` passes `-c user.name/user.email` (+ signing), resets and sets `credential.helper` to `git-vault credential --vault X`, and sets `GIT_SSH_COMMAND="ssh -i KEY -o IdentitiesOnly=yes"`. The helper answers only for the vault's host. Tokens are fetched from the backend at that moment and never written to disk by git-vault (except by the `file` backend, encrypted).

## Development

```bash
cargo test            # unit + integration tests (mock GitHub/GitLab/Gitea servers, fake pass/op/secret-tool)
cargo build --release
```

`docs/index.html` is the landing page (GitHub Pages: source `main` / `/docs`).

## Releasing

1. Bump `version` in `Cargo.toml`, commit, tag `vX.Y.Z`, push the tag. `.github/workflows/release.yml` builds macOS/Linux binaries and publishes the release the installer downloads.
2. `packaging/update-formula.sh vX.Y.Z` pins `packaging/homebrew/git-vault.rb` to that tag. Copy it to `Formula/git-vault.rb` in a `chungquantin/homebrew-tap` repo and push. Until then `brew install --HEAD` tracks `main`.

## Author

GitHub: [chungquantin](https://github.com/chungquantin)
