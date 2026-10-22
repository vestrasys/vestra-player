# CI（GitHub Actions）

方案与取舍见 [ci-research.md](ci-research.md) §10.1–10.5；这里只写现在实际怎么跑。检查命令与 [community-workflow.md](community-workflow.md) §6.1 一致。

## 文件

| 文件 | 作用 |
| --- | --- |
| `.github/workflows/ci.yml` | 入口：push 到 main 以外的任意分支（不含 tag）、pull_request、手动触发（`full`）。只做编排 |
| `.github/workflows/main.yml` | 入口：push 到 main。全部检查、两个系统都跑；只有它（和 nightly）写入 cargo 缓存 |
| `.github/workflows/nightly.yml` | 每天 01:17 UTC 与手动：RustSec 公告每天查；main 在 24 小时内有新提交才跑两个系统的 build-test 和未签名包 |
| `.github/workflows/release.yml` | tag `v*` 与手动（`dry_run`，默认开）：核对 tag 与 `src/host/Cargo.toml` 版本，同一套检查（不用缓存），release 构建，GitHub 草稿 release。签名是 S7 的 TODO job（`environment: release`） |
| `_static.yml` | 可复用：ubuntu 上的全部静态检查，是 Windows/macOS 的闸门 |
| `_build-test.yml` | 可复用（`os`）：整个 `src/host` workspace 的 clippy、test；Windows 另跑 rustdoc |
| `_package.yml` | 可复用（`os`）：`cargo build --release` 并上传未签名产物（S7 再做 .app/安装包与签名） |
| `_audit.yml` | 可复用：`cargo deny check advisories` |
| `.github/actions/setup-rust` | 按 `rust-toolchain.toml` 安装工具链；`Swatinem/rust-cache`，只在 main 上保存 |
| `.github/actions/setup-node` | Node 26 + npm 缓存；`src/` 和带 lockfile 的插件（nes、workshop）各 `npm ci` |
| `.github/actions/setup-libmpv` | macOS：`brew install mpv`。Windows：固定版本的 shinchiro mpv-dev（校验 SHA-256，缓存），从 `libmpv-2.dll` 的导出表生成 MSVC 用的 `mpv.lib`。设 `MPV_LIBRARY_DIR`（rsmpv-sys 的 build.rs 读它），DLL 目录加进 PATH |
| `.github/scripts/plan.sh` | ci.yml 的 plan：按改动的路径决定跑什么 |
| `rust-toolchain.toml`、`src/host/deny.toml`、`_typos.toml`、`.github/zizmor.yml` | 工具配置 |
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
│             test:sdk · plugins:build · plugin -- test --all · sdk:sync-slint
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
