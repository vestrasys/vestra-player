#!/bin/sh
# 实验预览：启动已构建播放器并预载全部插件，不代表成品安装或分发方案。
set -eu
experiment_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$experiment_root/scripts/env.sh"
if [ ! -x "$VESTRA_HOST" ]; then
    echo '缺少已构建播放器；请先运行 ./src/scripts/check.sh 或指定 VESTRA_HOST。' >&2
    exit 1
fi
for dependency in node ffmpeg ffprobe ffplay; do
    command -v "$dependency" >/dev/null 2>&1 || { echo "缺少 $dependency" >&2; exit 1; }
done

preview_executable=$VESTRA_HOST
if [ "$(uname -s)" = Darwin ]; then
    # 直接使用 binary 作为 bundle executable，保证原生窗口和系统对话框可被识别。
    preview_bundle="$repository_root/.cache/Vestra Interactive.app"
    preview_executable="$preview_bundle/Contents/MacOS/vestra"
    mkdir -p "$preview_bundle/Contents/MacOS"
    if ! cmp -s "$VESTRA_HOST" "$preview_executable"; then
        cp "$VESTRA_HOST" "$preview_executable"
        chmod +x "$preview_executable"
    fi
    node --input-type=module - "$preview_bundle/Contents/Info.plist" "$PATH" <<'NODE'
import { writeFileSync } from 'node:fs';
const [plist, path] = process.argv.slice(2);
const escape = value => value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
writeFileSync(plist, `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>local.vestra.interactive</string>
<key>CFBundleName</key><string>Vestra Interactive</string>
<key>CFBundleDisplayName</key><string>Vestra Interactive</string>
<key>CFBundleExecutable</key><string>vestra</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>NSHighResolutionCapable</key><true/>
<key>LSEnvironment</key><dict><key>PATH</key><string>${escape(path)}</string><key>SLINT_BACKEND</key><string>winit-software</string></dict>
</dict></plist>
`);
NODE
fi

set --
for manifest in "$experiment_root"/plugins/*/manifest.json; do
    [ -f "$manifest" ] || continue
    [ "$manifest" != "$experiment_root/plugins/file-playback/manifest.json" ] || continue
    set -- "$@" --plugin "$manifest"
done
# 基础播放面板最后加载，启动后先显示“打开 MP4”、暂停和进度条。
set -- "$@" --plugin "$experiment_root/plugins/file-playback/manifest.json"
export SLINT_BACKEND=${SLINT_BACKEND:-winit-software}
exec "$preview_executable" "$@"
