#!/bin/sh
# 实验素材生成器：用于观察播放与高斯模糊，不是产品代码。
set -eu
experiment_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
sample_output=${1:-"$experiment_root/samples/sample.mp4"}
mkdir -p "$(dirname -- "$sample_output")"
# -n 防止覆盖用户已有文件。
ffmpeg -hide_banner -loglevel error -n \
    -f lavfi -i 'testsrc2=size=960x540:rate=30:duration=12' \
    -f lavfi -i 'sine=frequency=440:sample_rate=48000:duration=12' \
    -c:v libx264 -preset ultrafast -pix_fmt yuv420p \
    -c:a aac -shortest -movflags +faststart "$sample_output"
printf '实验 MP4 已生成：%s\n' "$sample_output"
