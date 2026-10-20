#!/bin/sh
# git-vault installer.
#   curl -fsSL https://raw.githubusercontent.com/chungquantin/git-vault/main/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --shell      # also add shell integration to your rc file
#   ./install.sh                                      # from a clone: builds with cargo and symlinks
# Env: PREFIX (default ~/.local/bin), GIT_VAULT_VERSION (tag like v0.2.0, default latest release),
#      GIT_VAULT_BASE_URL (override the release download base, for mirrors/tests)
# Order: prebuilt release binary -> `cargo install` from source if no release matches.
set -eu

REPO="chungquantin/git-vault"
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
have() { command -v "$1" >/dev/null 2>&1; }

if [ "$UNINSTALL" = 1 ]; then
  rm -f "$DEST/git-vault"
  say "removed $DEST/git-vault"
  say "remove this line from your shell rc if present: $EVAL_LINE"
  say "vault data stays in \${GIT_VAULT_HOME:-~/.config/git-vault} and your secret backend (service git-vault)"
  exit 0
fi

have git || die "git not found"
mkdir -p "$DEST"

target() {
  os="$(uname -s)"; arch="$(uname -m)"
  case "$os/$arch" in
    Darwin/arm64) echo aarch64-apple-darwin ;;
    Darwin/x86_64) echo x86_64-apple-darwin ;;
    Linux/x86_64|Linux/amd64) echo x86_64-unknown-linux-musl ;;
    Linux/aarch64|Linux/arm64) echo aarch64-unknown-linux-musl ;;
    *) echo "" ;;
  esac
}

sha256_of() {
  if have shasum; then shasum -a 256 "$1" | cut -d' ' -f1
  elif have sha256sum; then sha256sum "$1" | cut -d' ' -f1
  else echo ""; fi
}

MODE=""

# 1. Dev mode: executed as a real file from a clone -> build + symlink.
case "$0" in
  */install.sh|install.sh)
    here="$(cd "$(dirname "$0")" && pwd)"
    if [ -f "$here/Cargo.toml" ]; then
      have cargo || die "cargo not found (install Rust: https://rustup.rs)"
      say "building from $here"
      (cd "$here" && cargo build --release --locked 2>&1 | tail -n 3)
      ln -sf "$here/target/release/git-vault" "$DEST/git-vault"
      say "linked $DEST/git-vault -> $here/target/release/git-vault"
      MODE=linked
    fi
    ;;
esac

# 2. Prebuilt release binary.
if [ -z "$MODE" ]; then
  T="$(target)"
  if [ -n "$T" ] && have curl && have tar; then
    if [ -n "${GIT_VAULT_BASE_URL:-}" ]; then
      BASE="$GIT_VAULT_BASE_URL"
    elif [ -n "${GIT_VAULT_VERSION:-}" ]; then
      BASE="https://github.com/$REPO/releases/download/$GIT_VAULT_VERSION"
    else
      BASE="https://github.com/$REPO/releases/latest/download"
    fi
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    say "downloading $BASE/git-vault-$T.tar.gz"
    if curl -fsSL "$BASE/git-vault-$T.tar.gz" -o "$tmp/gv.tar.gz" && curl -fsSL "$BASE/git-vault-$T.tar.gz.sha256" -o "$tmp/gv.sha256"; then
      want="$(cut -d' ' -f1 < "$tmp/gv.sha256")"
      got="$(sha256_of "$tmp/gv.tar.gz")"
      [ -n "$got" ] || die "no shasum/sha256sum available to verify the download"
      [ "$want" = "$got" ] || die "checksum mismatch (expected $want, got $got)"
      tar -xzf "$tmp/gv.tar.gz" -C "$tmp" git-vault
      install -m 755 "$tmp/git-vault" "$DEST/git-vault"
      say "installed $DEST/git-vault (checksum verified)"
      MODE=binary
    else
      say "no prebuilt binary for $T at $BASE"
    fi
  fi
fi

# 3. Source build fallback.
if [ -z "$MODE" ]; then
  have cargo || die "no prebuilt binary and cargo not found. Install Rust (https://rustup.rs) or Homebrew (brew install chungquantin/tap/git-vault)"
  say "building from source with cargo (a few minutes)"
  root="$(mktemp -d)"
  cargo install --git "https://github.com/$REPO" --locked --root "$root" git-vault 2>&1 | tail -n 3
  install -m 755 "$root/bin/git-vault" "$DEST/git-vault"
  rm -rf "$root"
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
