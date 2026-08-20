#!/bin/sh
set -eu

repo="skalenetwork/reef"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64) target="aarch64-unknown-linux-gnu" ;;
  Darwin-arm64) target="aarch64-apple-darwin" ;;
  *) echo "unsupported platform: $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac

if [ -n "${REEF_VERSION:-}" ]; then
  base="https://github.com/$repo/releases/download/v$REEF_VERSION"
else
  base="https://github.com/$repo/releases/latest/download"
fi

bin_dir="${REEF_INSTALL:-$HOME/.local/bin}"
asset="reef-$target.tar.gz"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$base/$asset" -o "$tmp/$asset"

if curl -fsSL "$base/checksums.sha256" -o "$tmp/checksums.sha256" 2>/dev/null; then
  if command -v sha256sum >/dev/null 2>&1; then sum="sha256sum"; else sum="shasum -a 256"; fi
  (cd "$tmp" && grep " $asset\$" checksums.sha256 | $sum -c - >/dev/null)
else
  echo "warning: no checksums published for this release, skipping verification" >&2
fi

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$bin_dir"
install -m 755 "$tmp/reef" "$bin_dir/reef"

echo "reef installed to $bin_dir/reef"
case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) echo "note: $bin_dir is not on your PATH" >&2 ;;
esac
echo "next: reef doctor"
