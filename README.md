# git-vault

Multiple git identities + credentials, selected per command, `aws-vault` style.

## Install

```bash
./install.sh
echo 'eval "$(git-vault shell-init)"' >> ~/.zshrc
```

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
