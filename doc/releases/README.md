# 发布说明与发版步骤

本目录放每个版本的发布说明：`v<版本>.md`，例如 `v0.3.0.md`。`release.yml` 把它原样作为 GitHub release 的正文；**推 tag 时这个文件不存在，release 就失败**（dry run 只给 warning）。

安装包发在本仓库自己的 GitHub Release（协调者决定 14）。仓库公开之前只做内部测试版，release 都是草稿，标题注明「unsigned test build」。

## 发版步骤

1. **确认依赖固定**：`src/packaging/deps.json` 的 `release` 指向一个存在的 `deps-*` release；要升级 libmpv/FFmpeg 就先跑 `deps.yml` 并提交新的 `deps.json`（[ci.md「打包」](../process/ci.md#打包)）。Node 补丁版本用 `node src/packaging/fetch-node.mjs --pin <版本>` 升级（主版本不变）。
2. **升版本号**：`src/host/Cargo.toml` 的 `version`（`cargo build` 会同步 `Cargo.lock`）。改过的内置插件也要升各自的版本，否则用户机器上不会更新（`src/scripts/stage-plugins.mjs` 的说明）。
3. **写发布说明** `doc/releases/v<版本>.md`：面向用户，中文；新功能、修复、已知问题；未签名测试版写上安装方法（见下）。
4. **dry run**：合并到 main 以后，
   ```
   gh workflow run release.yml --repo vestrasys/vestra-player --ref main -f dry_run=true
   ```
   全套检查、两个系统的安装包、`SHA256SUMS`，不签名、不建 release。产物在这次运行的 artifacts 里（`vestra-unsigned-macos`、`vestra-unsigned-windows`、`vestra-unsigned-SHA256SUMS`），用 `gh run download <run id>` 下载（只保留 1 天）。按 [packaging-proposal.md](../architecture/packaging-proposal.md) 末尾的清单在真机上试。
5. **打 tag**：
   ```
   git tag -a v<版本> -m "Vestra Player <版本>" && git push origin v<版本>
   ```
   `release.yml` 核对 tag 与版本、发布说明存在，跑全套检查、打包，`sign-*` 两个 TODO job 要在 `release` environment 里批准，最后建**草稿** release：资产是 `VestraPlayer-<版本>-macos-arm64.dmg`、`…-macos-arm64.app.zip`、`…-windows-x64-setup.exe`、`…-windows-x64-portable.zip` 和 `SHA256SUMS`。
6. **检查草稿**，确认无误后在 GitHub 上发布（或 `gh release edit v<版本> --draft=false`）。

失败时修好再推一个新的补丁版本 tag；不要移动已经推过的 tag。

## 未签名测试版怎么装

- macOS（14 以上，arm64；当前依赖包要求 15 以上，见 ci.md）：打开 DMG，把 Vestra Player 拖进「应用程序」。第一次打开会被 Gatekeeper 拦下：到「系统设置 → 隐私与安全性」点「仍要打开」，或者执行 `xattr -dr com.apple.quarantine "/Applications/Vestra Player.app"`。用 `gh run download` / `gh release download` 下载的文件没有隔离属性，不会被拦。
- Windows 10/11 x64：运行 `…-setup.exe`，SmartScreen 提示时点「更多信息 → 仍要运行」。安装在 `%LOCALAPPDATA%\Programs\Vestra Player`，不需要管理员；卸载时默认保留 `%APPDATA%\Vestra` 里的插件和设置。也可以解压便携 zip 直接运行 `vestra-host.exe`。

## 签名（以后）

`release.yml` 的 `sign-macos`、`sign-windows` 现在只是占位。接证书时需要的东西见方案 §8；bundle id 是 `com.vestraplayer.player`，正式签名发布以后不能再改。
