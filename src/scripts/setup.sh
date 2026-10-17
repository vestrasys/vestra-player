#!/bin/sh
# 实验依赖安装；每个插件保留独立锁文件，与最终发行安装方式可能差异很大。
set -eu
experiment_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
npm ci --prefix "$experiment_root"
for package_file in "$experiment_root"/plugins/*/package.json; do
    [ -f "$package_file" ] || continue
    npm ci --prefix "$(dirname -- "$package_file")"
done
