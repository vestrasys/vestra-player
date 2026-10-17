#!/bin/sh
# 实验验证脚本：真实插件/宿主/媒体边界；与最终成品 API 可能相差很大。
set -eu
experiment_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$experiment_root/scripts/env.sh"
cd "$experiment_root"
npm run typecheck
npm run boundaries
npm run format:check
cargo fmt --manifest-path host/Cargo.toml --check
cargo build --manifest-path host/Cargo.toml
cargo test --manifest-path host/Cargo.toml
cargo clippy --manifest-path host/Cargo.toml -- -D warnings
"$VESTRA_HOST" --check-ui
npm test
check_dir=$(mktemp -d "${TMPDIR:-/tmp}/vestra-backend-check.XXXXXX")
trap 'rm -rf "$check_dir"' EXIT HUP INT TERM
rustc --edition=2024 --test scripts/backend-test.rs -o "$check_dir/backend-test"
"$check_dir/backend-test" --nocapture
if [ "${1:-}" = --gui ]; then
    node tools/probe-gui.ts
fi
