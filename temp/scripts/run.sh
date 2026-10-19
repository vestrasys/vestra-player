#!/bin/sh
# 实验启动器：仅供 experimental1 使用，与最终产品/API 可能相差很大。
set -eu
experiment_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$experiment_root/scripts/env.sh"
if ! command -v cargo >/dev/null 2>&1; then
    machine_arch=$(uname -m)
    [ "$machine_arch" != arm64 ] || machine_arch=aarch64
    local_toolchain="$HOME/.rustup/toolchains/stable-$machine_arch-apple-darwin/bin"
    if [ -x "$local_toolchain/cargo" ]; then
        PATH="$local_toolchain:$PATH"
        export PATH
    else
        echo '需要 Rust 1.92+ 与 Cargo；请先将它们加入 PATH。' >&2
        exit 1
    fi
fi
rustc --version | node --input-type=module -e 'let s=""; for await (const chunk of process.stdin) s+=chunk; const m=s.match(/rustc (\d+)\.(\d+)/); if (!m || Number(m[1]) < 1 || (Number(m[1]) === 1 && Number(m[2]) < 92)) { console.error("Slint 1.18.1 需要 Rust 1.92+；当前：" + s.trim()); process.exit(1); }'
node --input-type=module -e 'const [major, minor] = process.versions.node.split(".").map(Number); if (major < 22 || (major === 22 && minor < 18)) { console.error("需要 Node.js 22.18+"); process.exit(1); }'
for dependency in ffmpeg ffprobe ffplay; do
    command -v "$dependency" >/dev/null 2>&1 || { echo "缺少 $dependency" >&2; exit 1; }
done
exec cargo run --manifest-path "$experiment_root/host/Cargo.toml" -- "$@"
