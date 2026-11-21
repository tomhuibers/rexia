#!/usr/bin/env bash
# Evaluate cache keys without compiling. Never mutate the working checkout.
set -euo pipefail
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
printf '\n// Cache-key regression probe.\n' >> "$tmp/src/main.rs"
[[ $(deps) == "$before_deps" ]]
[[ $(package) != "$before_package" ]]
printf '\n// Patched dependency cache-key regression probe.\n' >> "$tmp/vendor/ghostty-vt/src/lib.rs"
[[ $(deps) != "$before_deps" ]]
echo 'PASS: application edits reuse dependency artifacts; patched dependency edits invalidate them'
