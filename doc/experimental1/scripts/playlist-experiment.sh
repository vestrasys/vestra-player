#!/bin/sh
# 实验版本：真实窗口自动验证，不是最终安装流程或成品 API。
set -eu
experiment_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
sample_path="$experiment_root/samples/sample.mp4"
if [ ! -f "$sample_path" ]; then
    "$experiment_root/scripts/make-sample.sh"
fi
mkdir -p "$experiment_root/evidence"
SLINT_BACKEND=${SLINT_BACKEND:-winit-software}
export SLINT_BACKEND
exec "$experiment_root/scripts/run.sh" --experiment-playlist "$sample_path" \
    --report "$experiment_root/evidence/playlist-window-report.json"
