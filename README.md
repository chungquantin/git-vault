# git-vault

Multiple git identities + credentials, selected per command, `aws-vault` style.

## Install

```bash
# Homebrew (needs the tap, see "Releasing" below)
brew install chungquantin/tap/git-vault

# curl (add `sh -s -- --shell` to also wire your shell rc)
curl -fsSL https://raw.githubusercontent.com/chungquantin/git-vault/main/install.sh | sh

# from a clone (symlinks, handy for development)
./install.sh
```

Then enable the shell function and check the setup:

```bash
echo 'eval "$(git-vault shell-init)"' >> ~/.zshrc && source ~/.zshrc
git-vault doctor
```

Needs Python 3.8+ and git. `PREFIX=` changes the install dir (default `~/.local/bin`),
`GIT_VAULT_VERSION=v0.1.0` pins a tag, `install.sh --uninstall` removes it.

## Use

```bash
git add vault --name work --user-name "Me" --email me@corp.com --username me   # prompts for token
git add vault --name oss  --user-name "Me" --email me@oss.dev --ssh-key ~/.ssh/id_oss

git commit --vault work -m "msg"
git push   --vault work
git clone  --vault oss git@github.com:me/repo.git

GIT_VAULT=work git pull        # env alternative
git vault list | show <n> | remove <n>
```

## Two (or more) accounts

One vault per account; each does its own browser sign-in:

```bash
git add vault --name personal --user-name "Me" --email me@gmail.com --no-token
git vault login --name personal     # browser opens: sign in as the personal account

git add vault --name work --user-name "Me" --email me@corp.com --no-token
git vault login --name work         # browser opens: switch to the work account, approve (SSO happens here)

git push --vault personal
git push --vault work
```

`login` runs `gh auth login --web` in a throwaway config dir (needs `gh`), so it never touches
gh's own accounts and each vault gets exactly the account you approve. Alternatives:
`--from-gh [--gh-user U]` imports an account gh already has; `--client-id <ID>` (or
`GIT_VAULT_CLIENT_ID`) uses native OAuth device flow. The token is verified against the API
and stored in the Keychain; the tool warns on `X-GitHub-SSO` if the org needs authorization.

`git add vault` and `--vault` need the shell function from `shell-init`
(git can't be extended with new flags natively). `git vault <cmd>` works without it.

## Claude Code skill

`skills/git-vault/SKILL.md` teaches Claude Code to use vaults (raw `git-vault run ... --vault X`, vault selection, agent limits).

Install with [`npx skills`](https://github.com/vercel-labs/skills) (no clone needed):

```bash
npx skills add chungquantin/git-vault          # into the current project
npx skills add chungquantin/git-vault -g       # global, all projects
```

Or link it manually from a clone:

```bash
mkdir -p ~/.claude/skills && ln -sfn "$PWD/skills/git-vault" ~/.claude/skills/git-vault
```

## How it works

Per invocation only, nothing written to git config:

- `-c user.name / user.email` (+ `user.signingkey`, `commit.gpgsign` if set)
- `-c credential.helper=<git-vault credential --vault X>` for HTTPS; only answers for the vault's `--host`
- `GIT_SSH_COMMAND="ssh -i <key> -o IdentitiesOnly=yes"` for SSH remotes

Tokens live in macOS Keychain (service `git-vault`); elsewhere `0600` files under `~/.config/git-vault/secrets`.
Metadata: `~/.config/git-vault/vaults.json`. Override dir with `GIT_VAULT_HOME`, force file backend with `GIT_VAULT_BACKEND=file`.

Limits: one HTTPS host per vault; token briefly visible in `ps` during `security add-generic-password`.

## Landing page

`docs/index.html` is a static page with an interactive demo terminal. Preview locally with
`open docs/index.html`; to host it, enable GitHub Pages (Settings, Pages, source `main` / `/docs`).

## Releasing

1. Bump `VERSION` in `git-vault`, commit, tag `vX.Y.Z`, push the tag.
2. `packaging/update-formula.sh vX.Y.Z` pins `packaging/homebrew/git-vault.rb` to that tag.
3. Copy the formula to `Formula/git-vault.rb` in a `chungquantin/homebrew-tap` repo and push.
   Until a release exists, `brew install --HEAD chungquantin/tap/git-vault` tracks `main`.
