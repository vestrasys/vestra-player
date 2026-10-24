# CI（GitHub Actions）

方案与取舍见 [ci-research.md](ci-research.md) §10.1–10.5；这里只写现在实际怎么跑。检查命令与 [community-workflow.md](community-workflow.md) §6.1 一致。

## 文件

| 文件 | 作用 |
| --- | --- |
| `.github/workflows/ci.yml` | 入口：push 到 main 以外的任意分支（不含 tag）、手动触发（`full`）。只做编排。**没有 pull_request 触发**：同仓库 PR 直接显示 push 运行的检查（在同一个 head 提交上） |
| `.github/workflows/ci-macos.yml` | 入口：PR 的 macOS build-test，只在按需时跑：非草稿 PR opened/reopened/ready_for_review，或加上 `ci:macos` 标签（去掉再加一次即重跑）；PR 没改 host 路径时不触发 |
| `.github/workflows/main.yml` | 入口：push 到 main。和 ci.yml 同一个 plan；host 改动时 Windows、macOS 都跑（分支运行已在**同一个 tree** 上通过的系统除外，见下），只改文档时只跑 plan 和 ci-ok；只有它（和 nightly）写入 cargo 缓存 |
| `.github/workflows/nightly.yml` | 每天 01:17 UTC 与手动：RustSec 公告每天查；main 在 24 小时内有文档以外的改动才打两个系统的未签名包（用 cargo 缓存，包不发布）。main.yml 已经通过 tip 时不再跑 build-test（`main-green.sh`），否则先跑 |
| `.github/workflows/release.yml` | tag `v*` 与手动（`dry_run`，默认开）：核对 tag 与 `src/host/Cargo.toml` 版本、发布说明、依赖固定；main.yml 已经通过这个提交（或与它只差文档的祖先）时跳过 static 和 build-test，否则跑同一套检查（不用缓存；手动时 `checks` 强制跑）；打包始终不用缓存、从头构建；两个系统的未签名安装包、`SHA256SUMS`；tag 时建本仓库的草稿 release。签名是 TODO job（`environment: release`）。见下文「打包」和 [doc/releases/README.md](../releases/README.md) |
| `.github/workflows/deps.yml` | 只能手动：生成并固定原生依赖（libmpv、FFmpeg），发到本仓库的预发布 release `deps-<日期>`。见下文「打包」 |
| `_static.yml` | 可复用：ubuntu 上的全部静态检查，是 Windows/macOS 的闸门 |
| `_build-test.yml` | 可复用（`os`）：整个 `src/host` workspace 的 clippy、test；Windows 另跑 rustdoc |
| `_package.yml` | 可复用（`os`、`artifact`）：release 构建 → `src/packaging/bundle-*.mjs`（含许可证检查）→ 上传安装包 → 无界面冒烟测试 |
| `_audit.yml` | 可复用：`cargo deny check advisories` |
| `.github/actions/setup-rust` | 按 `rust-toolchain.toml` 安装工具链；`Swatinem/rust-cache`，只在 main 上保存 |
| `.github/actions/setup-node` | Node 26 + npm 缓存；`src/` 和带 lockfile 的插件（nes、workshop）各 `npm ci` |
| `.github/actions/setup-libmpv` | 按 `src/packaging/deps.json` 用 job token 下载本仓库 `deps-*` release 里的依赖包，校验 SHA-256，缓存，解压。设 `MPV_LIBRARY_DIR`（rsmpv-sys 的 build.rs 读它）和 `VESTRA_DEPS_DIR`（打包脚本读它）。macOS 另设 rpath（测试程序靠它找到 `@rpath/libmpv.2.dylib`），Windows 把 DLL 目录加进 PATH |
| `.github/scripts/plan.sh` | ci.yml、main.yml 的 plan：按改动的路径决定跑什么（见下） |
| `.github/scripts/main-green.sh` | release.yml、nightly.yml：main.yml 是否已经通过这个提交。先等这个提交上没跑完的 main.yml（release 最多 30 分钟，nightly 20 分钟，都在 Linux 上等），再看最近一次绿色的 main.yml：就是这个提交，或者是它的祖先且之间只改了 `doc/`、`*.md`，就算已检查 |
| `rust-toolchain.toml`、`src/host/deny.toml`、`_typos.toml`、`.github/zizmor.yml` | 工具配置 |
| `.gitattributes` | 所有检出都用 LF（Windows 默认会转成 CRLF，测试比较源码文本会失败） |
| `src/scripts/check-unsafe-policy.mjs` | unsafe 规则（见下） |

## ci.yml / main.yml 的阶段

```
push(非 main) / workflow_dispatch{full}      push(main)
│                                            │
plan [ubuntu]  改动路径 → docs_only, host, lock, run_windows, run_macos
│
├─ static [ubuntu]  (docs_only 时跳过)               ← 闸门
│    ├─ lint: typos · actionlint · zizmor · cargo fmt --check
│    │        cargo deny (licenses bans sources) · cargo shear · unsafe-policy
│    └─ ts:   npm run check · format:check · boundaries · portability --skip-host
│             licenses · test:sdk · plugins:build · plugin -- test <plugins> · sdk:sync-slint
│             git 工作区无改动
├─ audit [ubuntu]  (只在 ci.yml；lock: Cargo.lock / deny.toml 改了)  不阻塞，失败只出 warning
│
├─ windows [windows-2022, 2x]  needs static, if run_windows
│    clippy -D warnings · cargo test --workspace · cargo doc -D warnings
├─ macos   [macos-15, 10x]     needs static, if run_macos
│    clippy -D warnings · cargo test --workspace
│
└─ ci-ok  if: always()；任一 needs 为 failure/cancelled 就失败（跳过算通过）

pull_request{opened, reopened, ready_for_review, labeled ci:macos}，非草稿，改了 host 路径
└─ ci-macos.yml: macos [macos-15]  clippy · cargo test（static 和 Windows 由分支的 push 运行负责）
```

**改动从哪里算起**：plan.sh 用 API（`actions: read`）找这个 workflow 在这个分支上**最近一次成功运行**的提交（必须是 HEAD 的祖先，不是 HEAD 本身），`git diff` 它到 HEAD。没有这样的运行时：分支从它离开 main 的地方（`merge-base origin/main`）算，main 全跑。所以被 cancel-in-progress 取消的运行、被新 push 替换掉的 main 排队运行、失败的运行，它们的改动都会被下一次运行重新检查：tip 上绿色的 ci-ok 覆盖到上一次绿色为止的全部改动。

- `host` = 改了 `src/host/`、`src/runtime/`、`src/package*.json`、`rust-toolchain.toml`、`.gitattributes` 或 `.github/`（host 的测试会启动 Node 插件运行器）。ci-macos.yml 的 `paths` 与它一致。只改 `src/plugins/` 不算 host：插件测试在 static 的 ts job 里。
- `run_windows` = `host` 或 `full`。`run_macos` = `full`，或 main 上 `host`。分支 push 不跑 macOS：分支都在 Mac 上开发，`npm run ci:local` 本地就跑了 macOS（含 GPU 测试）；Windows 本地跑不了，所以分支上每次 host 改动都跑。
- 只改 `doc/` 或 `*.md`：只跑 plan 和 ci-ok（main 也一样）。
- `plugins`（ts job 跑哪些插件的测试）：全跑，或改了 `doc/`、`*.md`、`src/host/`、`src/plugins/<名字>/` 以外的任何路径（SDK 版本、`src/scripts/`、`.github/`……）时是 `all`；否则是改到的插件目录（有 `manifest.json` 的），一个都没有时是 `none`。插件只能 import 自己的文件和 SDK（`npm run boundaries`），别的插件改了影响不到它。release 调 `_static` 时不传，默认 `all`。18 个插件的测试一共约 220 秒，workshop 一个占 78 秒。
- **main 复用分支的结果**：main 上要跑 Windows / macOS 时，plan.sh 先查 ci.yml、ci-macos.yml 最近 50 次运行里 `head_commit.tree_id` 与 HEAD 的 tree 相同、且 `windows / windows`（或 `macos / macos`）job 成功的；有就不在 main 上再跑这个系统，日志里给出那次运行的链接。squash 合并一个已经包含最新 main 的 PR，main 的提交和 PR head 是同一个 tree，输入完全一样。ci-macos.yml 测的是 PR 的 merge 提交：head 包含 base 时它的 tree 就是 head 的，而只有这种情况 main 才会得到和 head 一样的 tree。**合并前先把 PR rebase 到最新 main**，main 上就基本不再重复编译。static 不复用（Linux，便宜；插件测试的范围按 main 自己的上次绿色算）。
- 每次 push 只有一个运行（以前有 PR 的分支 push 和 pull_request 各跑一次，PR 那次还多跑 macOS）。ci.yml 的 concurrency 组是 `ci-<ref>`，新 push 取消旧运行。
- main.yml 不取消正在跑的 main 运行（它要写缓存）。

**branch protection 只需要一个 required check：`ci-ok`。** 目前私有仓库在 Free 计划上不能开 branch protection（API 返回 403），`ci-ok` 只是给人看的；PR 页面上显示的是 push 运行的 `ci-ok`。

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
| `check-licenses.mjs` | `--static`（`npm run licenses`，`_static` 的 ts job）：about.toml 与 deny.toml 一致、清单里引用的仓库文件存在、带 npm 依赖的插件有 `licenses/`、有 `dependency-inventory.json` 或 `vendor/` 的插件在清单里有对应的 `inventory`/`vendor` 组件且其中每个包的许可证表达式允许、许可证文件都在、没有无主的许可证文件或 vendor 文件、Node 主版本与 CI 一致。工坊（Vestra Player AI）这类打包了几十个 npm 包的插件，清单只写 `inventory = "dependency-inventory.json"` 和 `vendor = "vendor"`，包列表由插件自己生成（`vendor-pi.mjs`、`vendor-sdk.mjs`），刷新 vendor 不用改 `third-party.toml`。`--bundle <根目录> --platform macos\|windows`（打包脚本最后一步）：包里每个文件都属于我们或某个组件，每个许可证文件都在，每个许可证表达式都在 `[policy].allowed` 里，插件包里带着打包进去的依赖的许可证（`inventory`/`vendor` 组件在 `.vxp` 里逐包再查一遍），`node --version` 等于固定版本 |
| `smoke.mjs` | 空数据目录、PATH 里没有 node 时无界面启动 `--headless --seconds 8`，要求 18 个内置插件都 `runtime.ready`（面板插件在无界面模式下报 `mounts require a native shell` 也算） |
| `macos/node.entitlements`、`macos/host.entitlements` | 以后签名用（Node 需要 JIT） |

**依赖包（deps.yml）**

- macOS：在 `macos-15` 上 `brew install mpv ffmpeg`，从 `libmpv.2.dylib`、`ffmpeg`、`ffprobe` 出发收集整个 dylib 闭包，install name 改成 `@rpath/<名字>`，ad-hoc 签名，各配方的许可证文件拷进 `licenses/<配方>/`，各配方源码包（`brew fetch -s`）另打一个 `-sources.tar`。
- **最低系统版本是 15.0，不是决定 3 的 14.0**：Homebrew 已经不再为 macOS 14 出 bottle（mpv、ffmpeg 等 45 个配方只有 sequoia 及更新的 bottle），在 `macos-14` 上要从源码编译整棵依赖树（含 deno、python、shaderc），不可行。`bundle-macos.mjs` 取 `components.json` 的 `min_macos` 与 14.0 中较大者作为 `LSMinimumSystemVersion`。要 14.0 只能自己编译 mpv/FFmpeg（方案 §6 的 B）。
- Windows：`deps.json` `upstream` 里固定的 shinchiro mpv-dev 与 BtbN `win64-gpl-shared`（FFmpeg n9.0），校验 SHA-256，取 `libmpv-2.dll`、头文件、`ffmpeg/{ffmpeg,ffprobe}.exe` 和 DLL，从 mpv 对应提交取许可证文件，用 MSVC 的 `dumpbin`/`lib` 生成 `mpv.lib`。另带 MSYS2 UCRT64 的 Vulkan 加载器 `vulkan-1.dll`（Apache-2.0）：`libmpv-2.dll` 在加载时就导入它，而 Windows 只有装了带 Vulkan 的显卡驱动才有这个文件（CI runner 上没有），缺了它播放器根本启动不了；它和 `libmpv-2.dll` 一起放在安装目录根上。
- 结果发到本仓库的预发布 release `deps-<UTC 日期>`（`GITHUB_TOKEN`，`contents: write`），附 `SHA256SUMS`、两份 `components-*.json` 和新的 `deps.json`。**把这份 `deps.json` 提交为 `src/packaging/deps.json`** 以后，CI 和打包才用新依赖。
- 只重建一个系统：`-f platforms=windows`（或 `macos`），另一个系统的资产从当前 `deps.json` 固定的 release 复制过来（校验 SHA-256），省掉 macOS runner。
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

**产物命名**：`VestraPlayer-<版本>-macos-arm64.dmg`、`…-macos-arm64.app.zip`（以后在 Linux 上签名要用）、`…-windows-x64-setup.exe`、`…-windows-x64-portable.zip`、`SHA256SUMS`。便携 zip 用 `7z -mx=5`：同样 445 MB 的库，`-mx=9` 要 8 倍时间（本机 89 秒对 11 秒；Windows runner 上 `-mx=9` 是 215 秒），zip 只小 1.5%。release 和 dry run 的 artifact 叫 `vestra-unsigned-<os>`，nightly 叫 `vestra-nightly-<日期>-<短哈希>-<os>`，都只保留 1 天（artifact 存储额度只有 500 MB；要长期保存的东西放 release 资产），nightly 不发 release。

**exe 图标和版本信息**：`src/host/build.rs` 在 MSVC 目标上用 `src/host/windows/resources.rs` 直接写出 `.res`（图标 `src/host/windows/vestra.ico` 加 VERSIONINFO），交给 link.exe 嵌入，不需要 rc.exe；输出与 `llvm-rc` 对同样的 `.rc` 逐字节相同。`vestra.ico` 由 `node packaging/icon.mjs --ico host/windows/vestra.ico` 生成并提交，`_static` 用 `--check` 确认它与 logo 一致：比较各条目的目录字段和解码后的像素，不比较字节（PNG 条目的 zlib 输出在 x64 Linux 和 arm64 macOS 上不同）；测试在 `src/packaging/tests/`（`npm run test:packaging`）。

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

计费倍数：Linux 1、Windows 2、macOS 10，每个 job 按分钟向上取整后再乘（plan、ci-ok、lint 各算 1 分钟；macOS 的 build-test 墙钟 1.1–1.7 分钟，取整成 2 分钟，就是 20）。私有仓库的额度按这些计费分钟算；公开仓库用标准 runner 不计费。

**缓存**：只有 main（和 nightly）写，分支和 PR 读 main 的（restore 正常命中）。2026-10-24 晚上仓库缓存共 7.2 GB / 10 GB，67 条：cargo `build-test` Windows 0.5–0.8 GB、macOS 0.77 GB，旧 Cargo.lock 的几条（约 3 GB）没人用，7 天后自动删；满 10 GB 时按最近使用淘汰，先走的是这些旧的，正在用的不会被挤掉。nightly 的打包另有一套 `package` 缓存（每个系统约 1 GB）。

**冷 / 热缓存实测（job 墙钟，分钟）**

| job | 热缓存 | 冷缓存（release，不用缓存） | 计费 |
| --- | --- | --- | --- |
| windows build-test | 3.8–4.8（restore 45 s、npm 25 s、clippy 30–45 s、test 40–85 s） | 8.7（clippy 3.8、test 3.8） | 8–10 / 18 |
| macos build-test | 1.1–1.2（restore 19 s、clippy 10 s、test 23 s） | 1.7（clippy 35 s、test 41 s） | 20 / 20（偶尔 2.2 → 30） |
| ts（static） | 4.5–5.5，其中插件测试 215–220 s | — | 5–6 |
| package windows | — | 21.0：`cargo build --release` 11.0、`cargo about` 1.3、7z `-mx=9` 3.6、Inno Setup 1.9、冒烟 1.1 | 42 |
| package macos | — | 4.4：release 构建 1.8、打包 1.7、冒烟 0.4 | 50 |

macOS 的缓存对计费几乎没有作用（冷热都是 2 分钟），Windows 的缓存每次省约 8。

**2026-10-24 13:00–20:30（ci/trim 之后，75 次运行）共 957 计费分钟**：Linux 417、Windows 190、macOS 350。20:20 以后的运行拿不到 runner（额度用完）。

| workflow / 事件 | 计费分钟 | 主要是 |
| --- | --- | --- |
| main / push（30 次） | 330 | macOS 120（6 次）、ts 93、Windows 52、plan + ci-ok 47。17:27–18:39 main 一直红（ts 失败），9 次 push 每次都按上次绿色重跑全部插件测试 |
| CI / push（33 次） | 325 | ts 148、Windows 78（9 次）、plan + ci-ok 66、lint 31 |
| CI macOS / pull_request（8 次） | 160 | 每个改 host 的 PR opened 时 20 |
| release / push v0.4.2 | 142 | 打包 92（Windows 42、macOS 50），重复的 static + build-test 44（main 已经通过同一提交） |

之前（2026-10-22 08:40 至 10-24 12:50，136 次运行）约 2470，其中 main 上只改文档的 push 约 690、pull_request 重复运行 310，已在 ci/trim 去掉。

**这一轮的改动（按上面 7.5 小时的运行回放估算）**

| 改动 | 省多少 |
| --- | --- |
| 插件测试只跑改到的插件（`plugins`） | 47 个 ts job 里 26 个不必全跑，共约 -77（ts 从 5–6 降到 1–3） |
| main 复用分支在同一 tree 上通过的 Windows / macOS | 这段时间 2 次（-36）；合并前都 rebase 时每个改 host 的 PR -20 到 -28 |
| release 跳过 main 已通过的 static + build-test | 每次 -44（v0.4.2：142 → 约 98），等 main 跑完最多花 Linux 几分钟 |
| 便携 zip `-mx=5` | 每次 Windows 打包约 -6 |
| 合计 | 这 7.5 小时约 957 → 800（-16%） |
| nightly：main 已通过 tip 时不跑 build-test；只改文档的日子不打包；打包用缓存 | 每个有改动的日子约 136 → 约 60（Windows 打包热缓存时估计 21 → 12 分钟，未实测） |

**每次 push 的估算（热缓存）**

| 场景 | 计费分钟 |
| --- | --- |
| 分支 push，只改文档 | 2 |
| 分支 push，只改一个插件 | 4–6（workshop 6） |
| 分支 push，改了 host（不改插件） | 13–15（plan 1、lint 1、ts 2、Windows 8–10、ci-ok 1） |
| 分支 push，改了 SDK 版本或 `.github/`（全部插件 + Windows） | 19 |
| PR opened（改了 host） | + macOS 20 |
| main push，只改文档 / 只改插件 | 2 / 4–6 |
| main push，改了 host，PR 合并前已 rebase、分支上 Windows 和 macOS 都过了 | 5 |
| main push，改了 host，没有可复用的结果 | 33–35 |
| tag release（main 已通过） | 约 98 |
| nightly（有改动，main 已通过） | 约 60 |

**现在要晚一点才知道的事**：分支上的 macOS 编译 / 测试错误（本地 `ci:local` 之外）要到 PR opened / 加 `ci:macos` 标签、或合进 main 之后才在 CI 里看到；只改文档的 main push 不跑 typos（分支 push 时跑）；release 的检查用的是 main 带缓存的那次结果（cargo 缓存只有依赖，不含 workspace 自己的 crate；包本身仍然不用缓存、从头构建并冒烟测试），要不带缓存重跑一遍就手动 `checks=true`。

**需要决定的选项（未做）**

按上面的强度（一天十几个 PR、几十次 push），即使省掉 16%，一个忙碌的工作日也要 700–800 计费分钟，私有仓库的月度额度（Free 2000、Team 3000）几天就用完。能根本解决的只有后三项。

| 选项 | 省多少 | 代价 |
| --- | --- | --- |
| PR 的 macOS 只在加 `ci:macos` 标签时跑（不再在 opened 时跑） | 每个改 host 的 PR -20（这段时间 -160） | macOS 问题要到合进 main 才在 CI 里看到（分支本来就在 Mac 上 `ci:local`） |
| main 上的 macOS 只留给 nightly | 每次 host main push 再 -20 | macOS 问题最多晚 24 小时 |
| nightly 打包改成每周一次（RustSec 仍每天） | 约 -60/天 → 每月约 -1500 | 打包 / 安装 / 冒烟问题最多晚一周发现；release 前可以手动 dry run |
| 删掉 release | 每次 -98 | 不建议：发布频率低，这是唯一从头构建、无缓存的打包 |
| macOS 自托管 runner（用户的 Mac） | macOS 全部（这段时间 350，占 37%）变成 0 计费 | Mac 必须在线，否则 job 一直排队；私有仓库里任何能 push 分支的人（包括代理）的代码都会在这台 Mac 上运行：要单独的 macOS 用户 / 虚拟机（tart），不碰钥匙串和个人文件；仓库公开前必须撤掉（公开仓库的 fork PR 能在自托管 runner 上跑代码）；GitHub 曾宣布对私有仓库的自托管 runner 收每分钟平台费，现状以计费页为准 |
| Windows 自托管 | Windows 全部（190，20%） | 需要一台 x64 Windows（小主机或云 VM，每月约 15–30 美元）：VS 2022 Build Tools（MSVC、Windows SDK）、Git for Windows（bash）、rustup、Node 26、7-Zip、Inno Setup 6、runner 服务、Defender 排除工作目录；安全问题同上。Apple 芯片上的 Windows ARM 虚拟机要模拟 x64，不推荐 |
| 仓库公开 | 标准 runner 全部免费（也能开 branch protection，`ci-ok` 成为真正的 required check） | 公开前要：全历史密钥扫描（gitleaks）；确认 `doc/` 里的内部讨论、round 文档可以公开；`deps-*` release 里的 GPL FFmpeg 要附对应源码（macOS 已有 `-sources.tar`，Windows 是上游构建的链接）；给 fork PR 加 `pull_request` 触发（只读 token、不写缓存，plan.sh 的 `actions: read` 可以用）；`release` environment 的保护规则；撤掉自托管 runner |
| 按量付费（设 spending limit） | — | 这 7.5 小时按实际分钟约 5–6 美元（Linux 417、Windows 95、macOS 35 分钟，按 GitHub 当前单价估，以计费页为准），一个月约 100–170 美元 |

还可以试（需要 CI 有额度时实测）：Windows 上用 Dev Drive（ReFS）放 `target` 和 cargo home、Defender 排除，通常能让 Windows 编译快 20–30%；把热缓存 macOS job 压到 60 秒以内（现在 66–72 秒）计费就从 20 变 10，但离边界太近，不可靠。sccache 帮助不大：rust-cache 已经是 full match，剩下的时间主要是测试本身和缓存下载。
