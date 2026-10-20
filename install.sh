#!/usr/bin/env bash
set -euo pipefail

src="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/git-vault"
dest="${PREFIX:-$HOME/.local/bin}"

chmod +x "$src"
mkdir -p "$dest"
ln -sf "$src" "$dest/git-vault"
echo "linked $dest/git-vault -> $src"

case ":$PATH:" in
  *":$dest:"*) ;;
  *) echo "warning: $dest not on PATH" ;;
esac

cat <<'EOF'

Add to ~/.zshrc (or ~/.bashrc) to enable `git add vault` and `git <cmd> --vault <name>`:

  eval "$(git-vault shell-init)"
EOF
