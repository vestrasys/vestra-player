#!/bin/sh
# 实验验证脚本：与最终产品的 API 和验证方式可能相差很大。
set -eu
experiment_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$experiment_root"
node --test tests/*.test.ts
if ! command -v rustc >/dev/null 2>&1; then
    machine_arch=$(uname -m)
    [ "$machine_arch" != arm64 ] || machine_arch=aarch64
    local_toolchain="$HOME/.rustup/toolchains/stable-$machine_arch-apple-darwin/bin"
    PATH="$local_toolchain:$PATH"
    export PATH
fi
check_dir=$(mktemp -d "${TMPDIR:-/tmp}/vestra-check.XXXXXX")
trap 'rm -rf "$check_dir"' EXIT HUP INT TERM
rustc --edition=2021 --test scripts/backend-test.rs -o "$check_dir/backend-test"
"$check_dir/backend-test" --nocapture
if [ "${1:-}" = --gui ]; then
    cargo check --manifest-path host/Cargo.toml
    cargo run --manifest-path host/Cargo.toml -- --check-ui
else
    printf '%s\n' '独立后端与插件测试通过；GUI 尚未由此脚本验证。加 --gui 检查 Rust 宿主及动态 Slint 源码编译，实际界面还需启动验证。'
fi
