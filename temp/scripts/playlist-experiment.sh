#!/bin/sh
# 实验版本：真实窗口自动验证，不是最终安装流程或成品 API。
set -eu
experiment_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
sample_path="$experiment_root/samples/sample.mp4"
if [ ! -f "$sample_path" ]; then
    "$experiment_root/scripts/make-sample.sh"
fi
evidence_root="$experiment_root/../doc/experimental1/evidence"
mkdir -p "$evidence_root"
SLINT_BACKEND=${SLINT_BACKEND:-winit-software}
export SLINT_BACKEND
exec "$experiment_root/scripts/run.sh" --experiment-playlist "$sample_path" \
    --report "$evidence_root/playlist-window-report.json"
