# 2026-10 轮：S7 打包与发布

开始于 2026-10-22，基线 main 13e2cb5 之后。用户指示「处理完 Dependabot 直接做 S7」。上一轮见 [2026-10-slint-store.md](2026-10-slint-store.md)。

**本文是这一轮的唯一进度来源。** 协调者上下文压缩后先读本文再继续。

## 目标（生态计划 S7、1.6 第 12 条）

1. 播放器自带 Node 24 LTS，不要求用户安装；SDK `engines` 与类型检查按这个版本调整。
2. macOS：`.app`（显示名 Vestra Player、新 logo 图标、内含 libmpv 及其依赖、内置插件、Node、第三方许可证），再做 `.dmg`。
3. Windows：安装包（含同样的内容）。
4. 第三方许可证：Rust 依赖用 cargo-about；libmpv/ffmpeg、Node、字体等非 Cargo 依赖用清单，CI 检查清单和包里实际文件一致。
5. 接进 `release.yml`：推 `v*` 标签 → 全套检查 → 各平台打包 → 公开发布地址上的 draft release。
6. 签名（Apple Developer ID、Windows 代码签名）这一轮只留接口，不接证书；先出不签名的测试版。

## 协调者决定

**2026-10-22（用户说「直接做 S7」，对上一条消息里的两项提议没有反对）**
1. 发布地址：新建公开仓库 `vestrasys/vestra-player-releases`，只放安装包与发布说明（播放器源码仓库保持私有）。到 release 步骤需要时再建。
2. 签名先不接，先出不签名的 draft release；签名步骤留成 `environment: release` 下的 TODO job。

**2026-10-22，步骤 7.2（方案中协调者能定的部分）**
3. macOS 第一版只出 arm64，最低 macOS 14.0（画中画要求 14）；依赖在 `macos-14` runner 上生成。Intel 版以后按需求再加。
4. 依赖来源：新增 `deps.yml`，一次生成 libmpv 等依赖，固定哈希，镜像到发布仓库；不在每次 release 时现装 Homebrew，Windows 也不依赖 shinchiro 会被删除的旧构建。
5. ffmpeg/ffprobe 两个平台都随包（导出、内嵌字幕、时长、缩略图都依赖它们）；宿主设置 `VESTRA_FFMPEG`/`VESTRA_FFPROBE`，SDK 不变。
6. Windows：Inno Setup 按用户安装（不要管理员）+ 便携 zip；卸载默认保留用户数据；exe 设为 GUI 子系统，Node 子进程不弹控制台；VC 运行库静态链接。
7. 关掉 mpv 内置的 Lua 脚本和 ytdl hook（网络视频由插件经 yt-dlp 解析，不需要 mpv 自己调用）。
8. 图标：先由 logo 自动合成圆角底板，以后有设计稿再替换。
9. 发布说明：`doc/releases/v<版本>.md`，缺文件则 release 失败。
10. 文件关联第一版不做（宿主还接不住系统传来的文件路径），单独排期。
11. 打包用自写脚本（不用 cargo-bundle/cargo-packager），DMG 用系统 `hdiutil`。
12. 补仓库根 LICENSE（GPL-3.0 全文）；运行时暂存时带上 SDK 的 LICENSE；nes 插件补 jsnes（Apache-2.0）许可证（交 nes 作者）。

**待用户决定**：Node 主版本（24 还是 26）、GPL 对应源码怎么提供、bundle id、发布仓库与凭据。

## 流程

自上而下（community-workflow §5）：维护者调研并写方案 → 协调者决定 → 维护者实现（宿主找内置 Node、资源路径、打包脚本、许可证）→ CI release 演练（dispatch dry_run）→ 协调者真机测试打包后的 `.app`（双击启动、插件全部激活、播放、商城安装）→ 合并。Windows 安装包在 CI 上构建并做冒烟测试，真机验收随 S6。

## 进度

- [x] 7.1 维护者调研与方案：[packaging-proposal.md](../../architecture/packaging-proposal.md)（试拼 .app 231 MB、DMG 76 MB；去掉 PATH 里的 node 后 18 个插件全部启动失败，证实必须内置 Node）
- [ ] 7.2 协调者决定
- [ ] 7.3 实现
- [ ] 7.4 release 演练（不签名）
- [ ] 7.5 真机测试 macOS 包
- [ ] 7.6 合并

## 记录

- 2026-10-22：SDK 的 Dependabot PR #1（TypeScript 5.9 → 7.0）关闭。理由：TypeScript 是 SDK 的运行时依赖，插件作者用它做类型检查；typedoc 0.28 只支持到 6.0。PR #2 让 Dependabot 不再提 TypeScript 大版本。升 7 以后作为单独的 SDK 版本评估。
- 2026-10-22：清理磁盘：删除 `src/host/target`（7.7 GB，旧的 portability 输出）和 `~/.cache/vestra-target` 下 Slint 1.16 时期的旧构建目录，空闲从 11 GB 到 50 GB。
