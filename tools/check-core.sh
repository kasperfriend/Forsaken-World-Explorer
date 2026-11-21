#!/bin/sh
# Dependency-free CPU-only checks for core modules.
# Does not compile GUI/GPU crates or shaders.
set -eu

cd "$(dirname "$0")/.."
RUSTC=${RUSTC:-rustc}
root=$(pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT HUP INT TERM

for module in settings modules; do
    "$RUSTC" --edition=2021 --test "src/$module.rs" -o "$work/$module"
    "$work/$module"
done

cat > "$work/self_test_harness.rs" <<EOF
#[path = "$root/src/settings.rs"]
mod settings;
#[path = "$root/src/modules.rs"]
mod modules;
#[path = "$root/src/self_test.rs"]
mod self_test;
EOF
"$RUSTC" --edition=2021 --test "$work/self_test_harness.rs" -o "$work/self_test_harness"
"$work/self_test_harness"
