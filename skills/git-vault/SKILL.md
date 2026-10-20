---
name: git-vault
description: Commit, push, pull or clone with a specific git identity/credential using git-vault. Use when the user mentions a vault name, "--vault", multiple git accounts, or asks to commit/push as a particular account (personal, work, etc.), or to add/sign in/manage a vault.
---

# git-vault

`git-vault` injects a named identity (author, HTTPS token, SSH key, signing key) into a single git command. Nothing is written to git config.

## Run git with a vault

The `git ... --vault X` shell function is NOT available in the agent's Bash tool. Use the raw form:

```bash
git-vault run <git args> --vault <name>
# e.g.
git-vault run commit --vault personal -m "msg"
git-vault run push --vault personal -u origin main
git-vault run clone --vault work git@github.com:org/repo.git
```

`--vault` may go anywhere before a `--`. `GIT_VAULT=<name>` is an alternative. If the binary is missing, run `./install.sh` from the repo (builds with cargo) or `brew install chungquantin/tap/git-vault`.

## Pick the vault

1. Use the vault the user named. Never guess between accounts.
2. If unnamed and more than one exists (`git-vault list`), ask which one.
3. `git-vault show <name>` prints metadata and `has_token`. It never prints secrets.

## Manage vaults

```bash
git-vault add --name N --user-name "Name" --email e@x.com [--host H --provider github|gitlab|gitea|bitbucket] [--ssh-key PATH] [--sign] [--extends PARENT] [--confirm none|terminal|dialog|touchid] [--no-token]
git-vault login --name N            # browser sign-in; --method device|gh|gh-import|token|password
git-vault rotate --name N           # new token stored first, then old one revoked
git-vault set --name N --email ...  # edit fields, token untouched
git-vault list | show N | remove N
git-vault rule add 'github.com/corp/*' --vault N     # URL rules for the credential helper
git-vault use N                     # pin the current repo to a vault (writes local git config)
git-vault install-helper            # plain `git push` picks a vault by rule/use
git-vault exec N -- <cmd>           # run any tool with the vault's token + identity in env
git-vault import gh|ssh [--apply]   # create vaults from gh accounts / ~/.ssh/config
git-vault doctor
```

- One vault = one account on one host. Token and SSH key must belong to the same account as the vault's email.
- HTTPS remotes use the token. `git@` remotes use `--ssh-key`; a vault without one cannot push over SSH.
- `--backend keychain|file|pass|op|libsecret` selects where tokens live.
- Do not use `add --force` just to edit a field; use `set`.

## Agent limits and safety

- Passphrase-protected SSH keys cannot be unlocked from the agent (no TTY, empty ssh-agent). If a push fails with `Permission denied (publickey)`, commit locally and hand the user the exact command to run in their terminal; suggest `ssh-add --apple-use-keychain <key>`.
- Browser `login` needs the user; do not attempt it unattended.
- A vault with `confirm` set prompts the human before releasing a token; if it is denied or no terminal exists, stop and tell the user.
- Never print, log or commit tokens or private keys. Do not read `~/.config/git-vault/` secrets or Keychain entries. `git-vault exec` puts the token in the child's environment; do not echo it.
- Pushing is outward-facing: only push when the user asked, to the remote they named.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `Permission denied (publickey)` | vault has no `ssh_key`, key not on that GitHub account, or passphrase locked |
| Prompts for username/password over HTTPS | vault has no token (`has_token: false`) or token's account lacks repo access; `git-vault login --name N` |
| `repository not found` | right credential, wrong account for that repo (private repos look like 404) |
| `must be authorized for SAML SSO` | authorize token/key for the org in GitHub settings |
| `git add vault` runs real git add | shell function missing: `eval "$(git-vault shell-init)"` in ~/.zshrc |
