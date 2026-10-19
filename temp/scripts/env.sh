#!/bin/sh
# 实验工具环境；与最终产品构建方案可能相差很大。由调用脚本设置 experiment_root。
repository_root=$(CDPATH= cd -- "$experiment_root/.." && pwd)
if [ -x "$repository_root/.cache/toolchains/cargo/bin/cargo" ]; then
    CARGO_HOME="$repository_root/.cache/toolchains/cargo"
    RUSTUP_HOME="$repository_root/.cache/toolchains/rustup"
    PATH="$CARGO_HOME/bin:$PATH"
    export CARGO_HOME RUSTUP_HOME
fi
if [ -d /opt/homebrew/opt/ffmpeg-full/bin ]; then
    PATH="/opt/homebrew/opt/ffmpeg-full/bin:$PATH"
fi
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$repository_root/.cache/probe-target"}
VESTRA_HOST=${VESTRA_HOST:-"$CARGO_TARGET_DIR/debug/vestra-slint-experimental1"}
export PATH CARGO_TARGET_DIR VESTRA_HOST
