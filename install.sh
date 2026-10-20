#!/bin/sh
# git-vault installer.
#   curl -fsSL https://raw.githubusercontent.com/chungquantin/git-vault/main/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --shell      # also add shell integration to your rc file
#   ./install.sh                                      # from a clone: symlinks (dev mode)
# Env: PREFIX (default ~/.local/bin), GIT_VAULT_VERSION (tag/branch, default main),
#      GIT_VAULT_BASE_URL (override download base, e.g. file:///path/to/clone)
set -eu

REPO="chungquantin/git-vault"
REF="${GIT_VAULT_VERSION:-main}"
BASE="${GIT_VAULT_BASE_URL:-https://raw.githubusercontent.com/$REPO/$REF}"
DEST="${PREFIX:-$HOME/.local/bin}"
EVAL_LINE='eval "$(git-vault shell-init)"'

SETUP_SHELL=0
UNINSTALL=0
for arg in "$@"; do
  case "$arg" in
    --shell) SETUP_SHELL=1 ;;
    --uninstall) UNINSTALL=1 ;;
    -h|--help) sed -n '2,8p' "$0" 2>/dev/null || true; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

say() { printf '%s\n' "$*"; }
die() { printf 'install: %s\n' "$*" >&2; exit 1; }

if [ "$UNINSTALL" = 1 ]; then
  rm -f "$DEST/git-vault"
  say "removed $DEST/git-vault"
  say "remove this line from your shell rc if present: $EVAL_LINE"
  say "vault data stays in \${GIT_VAULT_HOME:-~/.config/git-vault} and the Keychain (service git-vault)"
  exit 0
fi

command -v git >/dev/null 2>&1 || die "git not found"
command -v python3 >/dev/null 2>&1 || die "python3 not found (macOS: xcode-select --install)"
python3 -c 'import sys; sys.exit(sys.version_info < (3, 8))' || die "python >= 3.8 required"

mkdir -p "$DEST"

# Dev mode: executed as a real file from a clone -> symlink.
case "$0" in
  */install.sh|install.sh)
    here="$(cd "$(dirname "$0")" && pwd)"
    if [ -f "$here/git-vault" ]; then
      chmod +x "$here/git-vault"
      ln -sf "$here/git-vault" "$DEST/git-vault"
      say "linked $DEST/git-vault -> $here/git-vault"
      MODE=linked
    fi
    ;;
esac

if [ "${MODE:-}" != linked ]; then
  command -v curl >/dev/null 2>&1 || die "curl not found"
  tmp="$(mktemp)"
  trap 'rm -f "$tmp"' EXIT
  say "downloading $BASE/git-vault"
  curl -fsSL "$BASE/git-vault" -o "$tmp" || die "download failed"
  head -n 1 "$tmp" | grep -q python || die "downloaded file does not look like git-vault"
  install -m 755 "$tmp" "$DEST/git-vault"
  say "installed $DEST/git-vault"
fi

case ":$PATH:" in
  *":$DEST:"*) ;;
  *) say "warning: $DEST is not on PATH. Add: export PATH=\"$DEST:\$PATH\"" ;;
esac

if [ "$SETUP_SHELL" = 1 ]; then
  case "${SHELL:-}" in
    */zsh) rc="$HOME/.zshrc" ;;
    */bash) if [ "$(uname)" = Darwin ]; then rc="$HOME/.bash_profile"; else rc="$HOME/.bashrc"; fi ;;
    *) rc="" ;;
  esac
  if [ -z "$rc" ]; then
    say "unsupported shell (${SHELL:-unknown}); add manually: $EVAL_LINE"
  elif grep -qF 'git-vault shell-init' "$rc" 2>/dev/null; then
    say "shell integration already in $rc"
  else
    printf '\n# git-vault\n%s\n' "$EVAL_LINE" >> "$rc"
    say "added shell integration to $rc (restart your shell)"
  fi
else
  say ""
  say "Enable \`git add vault\` and \`git <cmd> --vault <name>\`: add to ~/.zshrc (or ~/.bashrc)"
  say "  $EVAL_LINE"
  say "or re-run with: sh -s -- --shell"
fi

"$DEST/git-vault" --version
