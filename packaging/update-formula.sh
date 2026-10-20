#!/bin/sh
# Pin the Homebrew formula to a released tag.
#   packaging/update-formula.sh v0.1.0
# Downloads the GitHub tag tarball, computes sha256, rewrites url/sha256 in
# packaging/homebrew/git-vault.rb. Then copy that file to
# chungquantin/homebrew-tap as Formula/git-vault.rb and push.
set -eu
tag="${1:?usage: update-formula.sh vX.Y.Z}"
formula="$(cd "$(dirname "$0")" && pwd)/homebrew/git-vault.rb"
url="https://github.com/chungquantin/git-vault/archive/refs/tags/$tag.tar.gz"
sha="$(curl -fsSL "$url" | shasum -a 256 | cut -d' ' -f1)"
[ -n "$sha" ] || { echo "could not fetch $url" >&2; exit 1; }
python3 - "$formula" "$url" "$sha" <<'PY'
import re, sys
path, url, sha = sys.argv[1:]
s = open(path).read()
s = re.sub(r'^  url ".*"\n  sha256 ".*"\n', '', s, flags=re.M)
s = s.replace('  head "', f'  url "{url}"\n  sha256 "{sha}"\n  head "', 1)
open(path, "w").write(s)
PY
echo "pinned $tag sha256=$sha"
