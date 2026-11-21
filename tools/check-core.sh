#!/bin/sh
# Dependency-free CPU-only checks for pure-std core modules.
# Does not compile GUI/GPU crates or shaders.
set -eu

cd "$(dirname "$0")/.."
RUSTC=${RUSTC:-rustc}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT HUP INT TERM

for module in settings logging modules; do
    "$RUSTC" --edition=2021 --test "src/$module.rs" -o "$work/$module"
    "$work/$module"
done
