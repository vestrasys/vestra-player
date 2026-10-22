# CI（GitHub Actions）

方案与取舍见 [ci-research.md](ci-research.md) §10.1–10.5；这里只写现在实际怎么跑。检查命令与 [community-workflow.md](community-workflow.md) §6.1 一致。

## 文件

| 文件 | 作用 |
| --- | --- |
| `.github/workflows/ci.yml` | 入口：push 到 main 以外的任意分支（不含 tag）、pull_request、手动触发（`full`）。只做编排 |
| `.github/workflows/main.yml` | 入口：push 到 main。全部检查、两个系统都跑；只有它（和 nightly）写入 cargo 缓存 |
| `.github/workflows/nightly.yml` | 每天 01:17 UTC 与手动：RustSec 公告每天查；main 在 24 小时内有新提交才跑两个系统的 build-test 和未签名包 |
| `.github/workflows/release.yml` | tag `v*` 与手动（`dry_run`，默认开）：核对 tag 与 `src/host/Cargo.toml` 版本、发布说明、依赖固定；同一套检查（不用缓存）；两个系统的未签名安装包、`SHA256SUMS`；tag 时建本仓库的草稿 release。签名是 TODO job（`environment: release`）。见下文「打包」和 [doc/releases/README.md](../releases/README.md) |
| `.github/workflows/deps.yml` | 只能手动：生成并固定原生依赖（libmpv、FFmpeg），发到本仓库的预发布 release `deps-<日期>`。见下文「打包」 |
| `_static.yml` | 可复用：ubuntu 上的全部静态检查，是 Windows/macOS 的闸门 |
| `_build-test.yml` | 可复用（`os`）：整个 `src/host` workspace 的 clippy、test；Windows 另跑 rustdoc |
| `_package.yml` | 可复用（`os`、`artifact`）：release 构建 → `src/packaging/bundle-*.mjs`（含许可证检查）→ 上传安装包 → 无界面冒烟测试 |
| `_audit.yml` | 可复用：`cargo deny check advisories` |
| `.github/actions/setup-rust` | 按 `rust-toolchain.toml` 安装工具链；`Swatinem/rust-cache`，只在 main 上保存 |
| `.github/actions/setup-node` | Node 26 + npm 缓存；`src/` 和带 lockfile 的插件（nes、workshop）各 `npm ci` |
| `.github/actions/setup-libmpv` | 按 `src/packaging/deps.json` 用 job token 下载本仓库 `deps-*` release 里的依赖包，校验 SHA-256，缓存，解压。设 `MPV_LIBRARY_DIR`（rsmpv-sys 的 build.rs 读它）和 `VESTRA_DEPS_DIR`（打包脚本读它）。macOS 另设 rpath（测试程序靠它找到 `@rpath/libmpv.2.dylib`），Windows 把 DLL 目录加进 PATH |
| `.github/scripts/plan.sh` | ci.yml 的 plan：按改动的路径决定跑什么 |
| `rust-toolchain.toml`、`src/host/deny.toml`、`_typos.toml`、`.github/zizmor.yml` | 工具配置 |
| `.gitattributes` | 所有检出都用 LF（Windows 默认会转成 CRLF，测试比较源码文本会失败） |
| `src/scripts/check-unsafe-policy.mjs` | unsafe 规则（见下） |

## ci.yml 的阶段

```
push(非 main) / pull_request / workflow_dispatch{full}
│
plan [ubuntu]  改动路径 → docs_only, host, lock, run_windows, run_macos
│
├─ static [ubuntu]  (docs_only 时跳过)               ← 闸门
│    ├─ lint: typos · actionlint · zizmor · cargo fmt --check
│    │        cargo deny (licenses bans sources) · cargo shear · unsafe-policy
│    └─ ts:   npm run check · format:check · boundaries · portability --skip-host
│             licenses · test:sdk · plugins:build · plugin -- test --all · sdk:sync-slint
│             git 工作区无改动
├─ audit [ubuntu]  (lock: Cargo.lock / deny.toml 改了)  不阻塞，失败只出 warning
│
├─ windows [windows-2022, 2x]  needs static, if run_windows
│    clippy -D warnings · cargo test --workspace · cargo doc -D warnings
├─ macos   [macos-15, 10x]     needs static, if run_macos
│    clippy -D warnings · cargo test --workspace
│
└─ ci-ok  if: always()；任一 needs 为 failure/cancelled 就失败（跳过算通过）
```

- `host` = 改了 `src/host/`、`src/runtime/`、`src/package*.json`、`rust-toolchain.toml`、`.gitattributes` 或 `.github/`（host 的测试会启动 Node 插件运行器）。
- `run_windows` = `host` 或 `full`。`run_macos` = `full`，或目标为 main 的 PR 且 `host`。
- 只改 `doc/` 或 `*.md`：只跑 plan 和 ci-ok。
- 同一个分支有 PR 时，push 和 pull_request 各跑一次（concurrency 组不同）；PR 那次多跑 macOS。
- main.yml：static → windows + macos → ci-ok，不取消正在跑的 main 运行（它要写缓存）。

**branch protection 只需要一个 required check：`ci-ok`。**

## 和 §6.1 的对应

§6.1 的每一条都在 CI 里，另外加了 typos、cargo-deny、cargo-shear、unsafe-policy、actionlint、zizmor。差别：

- `npm run portability` 在 CI 里只跑系统命令扫描（`--skip-host`）。它的宿主部分（在 macOS 上对 Windows 目标 `cargo check`）由 windows job 的真实编译、clippy、测试代替。
- `cargo clippy` / `cargo test` 用 `--workspace`，三个 unsafe crate 也检查、测试。
- `cargo test` 加 `--no-fail-fast`，一个测试程序失败后其余照跑。
- 需要 GPU 的测试（`unsafe/render` 的两个 OpenGL 测试）标了 `#[ignore]`：CI 的 macOS 虚拟机建不出 OpenGL 4.1 上下文。`ci:local` 用 `--include-ignored` 在本机跑它们。
- `cargo doc` 只在 Windows 上跑。

## unsafe 规则

- `src/host/Cargo.toml` 的 `[workspace.lints.rust] unsafe_code = "forbid"`，host 用 `[lints] workspace = true`。
- `src/host/unsafe/*` 三个 crate 不继承，各自写 `unsafe_op_in_unsafe_fn = "deny"`、`clippy::undocumented_unsafe_blocks = "deny"`、`clippy::missing_safety_doc = "deny"`。
- `npm run unsafe-policy`（任何系统都能跑，不编译）：workspace 成员的 lint 配置；所有已跟踪 `.rs` 文件（去掉注释和字符串）在 `src/host/unsafe/` 以外不得出现 `unsafe` 或 `allow(unsafe_code)`，cfg 掉的平台代码也查；unsafe crate 里每个 `unsafe {}` / `unsafe impl` 上方必须有 `// SAFETY:`。新增 unsafe crate 要改脚本里的 `UNSAFE_CRATES`。

## 打包

脚本都在 `src/packaging/`（Node 内置模块，不需要 npm 依赖），由 `_package.yml` 调用，本地也能跑。方案见 [packaging-proposal.md](../architecture/packaging-proposal.md)，决定见 [2026-10-s7-packaging.md](rounds/2026-10-s7-packaging.md)。

| 文件 | 作用 |
| --- | --- |
| `node.json` | 内置 Node 的版本和两个平台压缩包的 SHA-256，另有 Node 发布密钥环（`nodejs/release-keys` 固定提交）的 URL 和 SHA-256 |
| `fetch-node.mjs` | 下载 Node，用 `gpgv` 验 `SHASUMS256.txt` 的签名，核对它和 `node.json` 一致，再核对压缩包；只取出 `node`/`node.exe` 和 `LICENSE`。`--pin <版本>` 升级 Node：验签以后改写 `node.json`。主版本必须等于 CI 的 `setup-node`（`npm run licenses` 会查） |
| `deps.json` | 依赖包的固定：`release`（如 `deps-20261022`）、`assets`（文件名和 SHA-256）、`upstream`（Windows 上游构建的 URL 和 SHA-256，`deps.yml` 读它） |
| `collect-macos-deps.mjs`、`collect-windows-deps.mjs` | `deps.yml` 用：收集依赖、改 install name、写 `components.json`（每个库的版本、许可证、源码、文件、许可证文件；macOS 另有 `min_macos`） |
| `bundle-macos.mjs` | 拼 `Vestra Player.app`（布局见方案 §3.1）：主程序、`Frameworks/` 里的 dylib 闭包、`MacOS/{node,ffmpeg,ffprobe}`、图标、`Resources/{ui,runtime,plugins,licenses}`；把主程序对 libmpv 的引用改成 `@rpath`，检查闭包；由内到外 ad-hoc 签名（`--identity` 时用证书、Hardened Runtime 和 `macos/*.entitlements`）；许可证检查；`hdiutil` 生成 ULMO 格式的 DMG；`--zip` 另出 `.app.zip` |
| `bundle-windows.mjs`、`windows/vestra.iss` | 暂存安装目录（布局见方案 §4.2），许可证检查，便携 zip，Inno Setup 安装包（按用户安装，不要管理员，AppId `com.vestraplayer.player`，卸载时默认保留 `%APPDATA%\Vestra`） |
| `stage.mjs` | 两个系统共用：`ui`、`runtime`（`scripts/stage-runtime.mjs`）、内置插件（`scripts/stage-plugins.mjs`）、`licenses/`（`index.html` 总目录、`LICENSE`、`rust-crates.html`、`third-party/…`） |
| `icon.mjs` | 从 `src/assets/brand/logo.png` 生成图标：macOS 是 824 px 深色圆角底板加 logo 的 `.icns`，Windows 是 logo 本身的 `.ico`。纯 Node 实现，各系统输出相同 |
| `about.toml`、`about.hbs` | cargo-about 配置和 HTML 模板。`accepted` 必须与 `src/host/deny.toml` 的 `allow` 相同 |
| `third-party.toml` | 非 Cargo 组件清单：我们自己的文件（`[own]`）、各组件的文件、许可证和来源；libmpv/FFmpeg 闭包由包里的 `components.json` 展开 |
| `check-licenses.mjs` | `--static`（`npm run licenses`，`_static` 的 ts job）：about.toml 与 deny.toml 一致、清单里引用的仓库文件存在、带 npm 依赖的插件有 `licenses/`、Node 主版本与 CI 一致。`--bundle <根目录> --platform macos\|windows`（打包脚本最后一步）：包里每个文件都属于我们或某个组件，每个许可证文件都在，每个许可证表达式都在 `[policy].allowed` 里，插件包里带着打包进去的依赖的许可证，`node --version` 等于固定版本 |
| `smoke.mjs` | 空数据目录、PATH 里没有 node 时无界面启动 `--headless --seconds 8`，要求 18 个内置插件都 `runtime.ready`（面板插件在无界面模式下报 `mounts require a native shell` 也算） |
| `macos/node.entitlements`、`macos/host.entitlements` | 以后签名用（Node 需要 JIT） |

**依赖包（deps.yml）**

- macOS：在 `macos-15` 上 `brew install mpv ffmpeg`，从 `libmpv.2.dylib`、`ffmpeg`、`ffprobe` 出发收集整个 dylib 闭包，install name 改成 `@rpath/<名字>`，ad-hoc 签名，各配方的许可证文件拷进 `licenses/<配方>/`，各配方源码包（`brew fetch -s`）另打一个 `-sources.tar`。
- **最低系统版本是 15.0，不是决定 3 的 14.0**：Homebrew 已经不再为 macOS 14 出 bottle（mpv、ffmpeg 等 45 个配方只有 sequoia 及更新的 bottle），在 `macos-14` 上要从源码编译整棵依赖树（含 deno、python、shaderc），不可行。`bundle-macos.mjs` 取 `components.json` 的 `min_macos` 与 14.0 中较大者作为 `LSMinimumSystemVersion`。要 14.0 只能自己编译 mpv/FFmpeg（方案 §6 的 B）。
- Windows：`deps.json` `upstream` 里固定的 shinchiro mpv-dev 与 BtbN `win64-gpl-shared`（FFmpeg n9.0），校验 SHA-256，取 `libmpv-2.dll`、头文件、`ffmpeg/{ffmpeg,ffprobe}.exe` 和 DLL，从 mpv 对应提交取许可证文件，用 MSVC 的 `dumpbin`/`lib` 生成 `mpv.lib`。
- 结果发到本仓库的预发布 release `deps-<UTC 日期>`（`GITHUB_TOKEN`，`contents: write`），附 `SHA256SUMS`、两份 `components-*.json` 和新的 `deps.json`。**把这份 `deps.json` 提交为 `src/packaging/deps.json`** 以后，CI 和打包才用新依赖。
- 升级：改 `deps.json` 的 `upstream`（Windows），或直接重跑（macOS 取当时的 Homebrew），提交新的 `deps.json`。旧的 `deps-*` release 不要删：旧版本的构建靠它们复现。

```
gh workflow run deps.yml --repo vestrasys/vestra-player --ref main
```

**本地打包（macOS）**

```
cd src/host && MPV_LIBRARY_DIR=<deps>/lib cargo build --release   # 或者用 Homebrew 的 libmpv 构建
cd .. && VESTRA_DEPS_DIR=<deps> npm run package:macos -- --out <输出目录> --zip
node packaging/smoke.mjs --exe "<输出目录>/Vestra Player.app/Contents/MacOS/vestra-host" \
  --plugins "<输出目录>/Vestra Player.app/Contents/Resources/plugins"
```

`<deps>` 可以用 `gh release download <deps 标签> -p vestra-deps-macos-arm64.tar.xz` 下载后解压，也可以本机 `node packaging/collect-macos-deps.mjs --out <deps>`（用本机 Homebrew，最低系统版本跟着本机走，只适合自测）。需要 `brew install cargo-about`（或者 `--rust-licenses <html>` 跳过）。Windows 包在 macOS 上可以用 `--no-installer` 暂存和出 zip，安装包只能在 Windows 上编。

**产物命名**：`VestraPlayer-<版本>-macos-arm64.dmg`、`…-macos-arm64.app.zip`（以后在 Linux 上签名要用）、`…-windows-x64-setup.exe`、`…-windows-x64-portable.zip`、`SHA256SUMS`。release 和 dry run 的 artifact 叫 `vestra-unsigned-<os>`，nightly 叫 `vestra-nightly-<日期>-<短哈希>-<os>`，都只保留 1 天（artifact 存储额度只有 500 MB；要长期保存的东西放 release 资产），nightly 不发 release。

**冒烟测试的过渡状态**：宿主找自带 Node 的改动（S7 宿主分支）合并之前，`_package.yml` 的 `SMOKE_COMPAT=--node-on-path` 把包里 node 的目录放进 PATH；Windows 的冒烟测试 `continue-on-error`（旧宿主在 Windows 上找不到 `resources\`）。宿主分支合并后两处都删掉。

## 手动跑全量（含 macOS）

```
gh workflow run ci.yml --repo vestrasys/vestra-player --ref <分支> -f full=true
```

或在 Actions 页面选 CI → Run workflow → 勾选 full。`full=false` 的手动运行等于「全部改动」，跑 static + windows。

## 本地等价命令

在 `src/` 下：

```
npm run ci:local                  # static + 本机系统的 clippy/test/doc
npm run ci:local -- --static      # 只跑静态检查
npm run ci:local -- --advisories  # 另查 RustSec 公告
```

需要 `brew install typos-cli actionlint zizmor cargo-deny cargo-shear`。cargo 输出在 `$CARGO_TARGET_DIR`，未设时在系统临时目录的 `vestra-target/ci`。本地会完整跑 `portability`。Homebrew 的 Rust 不读 `rust-toolchain.toml`，版本以本机为准。

## 成本

计费倍数：Linux 1、Windows 2、macOS 10，每个 job 按分钟向上取整。缓存只有 main 写，feature 分支读 main 的缓存；main 还没跑过时分支都是冷缓存。

实测（2026-10-22，`ci/rework` 分支，全部冷缓存）：

| 运行 | job（分钟） | 墙钟 | 计费分钟 |
| --- | --- | --- | --- |
| push，只有 static（[39759385214](https://github.com/vestrasys/vestra-player/actions/runs/39759385214)） | plan 0.1 · lint 0.5 · ts 3.8 · ci-ok 0.1 | 4.1 | 7 |
| push，static + Windows（[39754674923](https://github.com/vestrasys/vestra-player/actions/runs/39754674923)） | 同上 + windows 8.0（clippy 3.4、test 3.3、doc 0.1） | 12.1 | 23 |
| 手动 full（[39760523366](https://github.com/vestrasys/vestra-player/actions/runs/39760523366)） | + audit 0.4 · windows 7.9 · macos 1.8（clippy 0.6、test 0.6） | 12.5 | 45 |

macOS（macos-15 arm64）编译比 Windows 快得多，按 10 倍计费仍是最贵的一项。
