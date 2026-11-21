#!/usr/bin/env bash
# Evaluate cache keys; --build also builds before and after the source edit.
# Never mutate the working checkout.
set -euo pipefail
if [[ $# -gt 1 || ( $# -eq 1 && $1 != --build ) ]]; then
  echo "Usage: $0 [--build]" >&2
  exit 2
fi
build=${1:-}
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cp -r "$root"/{flake.nix,flake.lock,Cargo.toml,Cargo.lock,nix,src,vendor,assets,resources} "$tmp/"
mkdir -p "$tmp/tests/fixtures"
cp "$root/tests/fixtures/program-icons.txt" "$tmp/tests/fixtures/"
chmod -R u+w "$tmp"
system=$(nix eval --impure --raw --expr builtins.currentSystem)
attr="path:$tmp#packages.$system.rexia"
deps() { nix eval --raw "$attr.cargoArtifacts.drvPath"; }
package() { nix eval --raw "$attr.drvPath"; }
before_deps=$(deps)
before_package=$(package)
if [[ $build == --build ]]; then nix build -L --no-link "$attr"; fi
printf '\n// Cache-key regression probe.\n' >> "$tmp/src/main.rs"
[[ $(deps) == "$before_deps" ]]
[[ $(package) != "$before_package" ]]
if [[ $build == --build ]]; then nix build -L --no-link "$attr"; fi
printf '\n// Patched dependency cache-key regression probe.\n' >> "$tmp/vendor/ghostty-vt/src/lib.rs"
[[ $(deps) != "$before_deps" ]]
echo 'PASS: application edits reuse dependency artifacts; patched dependency edits invalidate them'
