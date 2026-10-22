# 开源项目 GitHub Actions CI 结构调研 + Vestra Player CI 方案

调研日期：2026-10-22。所有结论都来自 `gh api repos/<o>/<r>/contents/.github/workflows/...` 拉下来的当前 HEAD 文件（原文缓存在 `scratchpad/wf/<owner>_<repo>/`）。文中路径都相对于对应仓库根目录。
标注【推断】的是我自己的判断，没有出现在被调研的文件里。

---

## 0. 一页结论

| 模式 | 谁在用（文件） | 对我们的价值 |
|---|---|---|
| **入口 workflow 只做编排，检查逻辑放进 `workflow_call` 可复用 workflow** | uv `ci.yml` → `check-*.yml`/`test.yml`；langgraph `ci.yml` → `_lint.yml`/`_test.yml`；slint `autofix.yaml` → `ci.yaml` → `build_and_test_reusable.yaml` | PR、main、release 共用同一套检查 |
| **先跑便宜的静态检查，再放行昂贵的 OS 矩阵**（`needs:` 当闸门） | tokio `ci.yml`：`basics` needs fmt/clippy/docs/minrust，其余约 40 个 job 都 `needs: basics`；slint `autofix.yaml`：`ci` needs `format_fix, lint_typecheck`；langgraphjs `ci.yml`：test needs lint/format/build | 私有仓库 macOS 按 10 倍计费，这条必须做 |
| **一个 plan/changes job 算出输出，后续 job 用 `if:` 按路径/标签开关** | uv `plan.yml`（路径 + `test:macos` 等 PR 标签）；slint `.github/ci_path_filters.yaml` + `dorny/paths-filter`；deno `pre_build`（只改文档、draft PR 就跳过） | 省钱的核心机制 |
| **PR 默认不跑昂贵 OS，main、tag 或打了标签才跑** | deno `ci.ts`：PR 没有 `ci-full` 标签时，把 macOS/Windows release 构建的 runner 换成 ubuntu 并跳过；uv `plan.yml`：`test:macos` 标签；slint `schedule_ci.yaml`：每天 3 次对有新提交的分支跑 `full=true` | 直接照搬 |
| **最后放一个汇总 job 给 branch protection 用**（`if: always()`，任一 needs 是 failure/cancelled 就失败） | slint `autofix.yaml` 的 `done`；langgraph `ci.yml` 的 `ci_success`；uv `ci.yml` 的 `required-checks-passed`；deno 的 `ci-status`；vite 的 `test-passed`/`test-failed` | 只设一个 required check |
| **release 前重新验证，并把构建和发布拆成两个 job**（发布 job 才拿 OIDC 或写权限） | langgraph `release.yml`（注释引用 pypa：build 与 publish 分 job）；uv `release.yml`；deno `npm_publish.ts`（先发到本地 Verdaccio 冒烟测试再发 npm） | 防止依赖投毒拿到发布凭据 |
| **签名放在 `environment` 里，只对 main 或 tag 生效** | deno：`environment: build` 只在 main/tag 上；uv `sign-release-binaries.yml`：`environment: release`；vite `publish.yml`：`environment: Release` | secrets 隔离 |
| **macOS 签名和公证放在 Linux 上做**（rcodesign + 云 HSM） | uv `sign-release-binaries.yml`：`sign-macos` 跑在 ubuntu 上，用 `astral-sh/github-actions/sign-macos`（rcodesign + Azure Key Vault + Apple 公证）；deno 用 `rcodesign sign` | 公证等待通常 5–30 分钟，放在 macOS runner 上要按 10 倍计费 |
| **Windows 签名用 Azure Trusted/Artifact Signing + OIDC** | deno `ci.ts`（`Azure/artifact-signing-action` 之后 `signtool verify /pa`）；uv `sign-windows` | 不需要 pfx 证书文件 |
| **npm 用 OIDC trusted publishing 或 provenance** | vite `publish.yml`（`id-token: write`，npm ≥ 11.5.2）；langgraphjs `release.yml`（changesets + `NPM_CONFIG_PROVENANCE: true`）；tauri `publish-cli-js.yml`（`NODE_AUTH_TOKEN: ''`，走 OIDC） | Repo B 直接用 |
| **Rust 供应链检查** | tokio `audit.yml`（每日）+ `pr-audit.yml`（PR 改了 `Cargo.toml` 才跑），`EmbarkStudios/cargo-deny-action`；uv `check-lint.yml`（`cargo deny check licenses/sources/bans` + cargo-shear + typos）；tauri `audit.yml`（rustsec/audit-check + pnpm audit，每日）、`supply-chain.yml`（cargo-vet） | 我们要随包附带第三方许可证，cargo-deny 的 licenses 检查是刚需 |
| **扫描 workflow 本身的安全问题** | uv `check-zizmor.yml`；vite `zizmor.yml`，另有 semgrep 每周跑 | 便宜，应该加 |
| **所有 action 按 SHA 固定** | uv、langgraph、vite、deno（`# gagen:pin` 自动把 tag 转成 SHA） | 发布路径上必须这样做 |

---

## 1. slint-ui/slint（Rust + Slint，目标平台很多）

### 1.1 文件与触发方式
- `autofix.yaml`：**真正的 PR 入口**。`on: pull_request (master, feature/*, pre-release/*) / workflow_dispatch / merge_group`，`concurrency: ci-${{ PR号 || ref }}`，`cancel-in-progress: true`，顶层 `permissions: contents: read`。
- `ci.yaml`：`on: workflow_call + workflow_dispatch`，输入 `full`（bool）和 `changes`（paths-filter 命中分组的 JSON 列表）。它自己**不监听 push 或 PR**。
- `schedule_ci.yaml`：`cron '18 4,11,18 * * *'`。遍历 `master|feature/*|pre-release/*` 分支，只要分支 tip 和上次 CI 跑的 sha 不同，就 `gh workflow run CI -f full=true`。触发用 GitHub App token，因为 GITHUB_TOKEN 发起的 dispatch 不会再触发 workflow_run。
- `notify_ci_failure.yaml`：`workflow_run` 监听 `CI` 和 `Nightly snapshot` 在 master 上失败后发通知。
- `schedule_nightly_snapshot.yaml`：`cron '18 2 * * *'`，24 小时内有提交才 dispatch `nightly_snapshot.yaml`（mode 取值 `full-nightly`/`website-only`）。
- `nightly_snapshot.yaml`：`workflow_dispatch(mode: release|full-nightly|website-only|test)`。nightly 和正式 release 走**同一个文件**。
- `pr_checks.yaml`：`pull_request_target`，注释写明 *"never checks out or runs code from the pull request"*，只读 PR 描述，检查贡献协议是否勾选。
- 复用组件：`build_and_test_reusable.yaml`、`node_test_reusable.yaml`、`python_test_reusable.yaml`、`build_docs.yaml`、`slint_tool_binary.yaml`、`wasm_*.yaml`、`cpp_package.yaml`、`upload_pypi*.yaml`（都是 `workflow_call`）；composite action 有 `.github/actions/setup-rust`、`install-linux-dependencies`、`codesign`、`install-sparkle`。

### 1.2 Job 图
```
autofix.yaml (PR / merge_group)
 ├─ format_fix      [ubuntu, 5min] paths-filter(.github/ci_path_filters.yaml) → outputs.changes
 │                   mise run ci:autofix:fix (cpp/py/rust/toml/ts 格式化, license 头, pnpm dedupe) → autofix-ci 提交建议
 ├─ lint_typecheck  [ubuntu, 5min] mise run ci:autofix:lint (REUSE, python, ts typecheck, knip) + cspell
 ├─ ci  needs[format_fix, lint_typecheck]  uses: ./ci.yaml  with{full:false, changes}
 │    ci.yaml 内每个 job:  if: inputs.full || contains(fromJSON(inputs.changes), '<group>')
 │     ├─ build_and_test (matrix: windows-2022@1.95[MSRV], macos-14@beta, ubuntu…) → uses build_and_test_reusable.yaml
 │     ├─ clippy [ubuntu] ./scripts/run_clippy.sh  (4 个 workspace 都 -D warnings)
 │     ├─ node_test_{linux,macos,windows} / python_test_* → uses *_reusable.yaml
 │     ├─ docs (only !full) → uses build_docs.yaml   (RUSTDOCFLAGS=-D warnings)
 │     ├─ cross_compile_checks / android / ios_cpp / esp-idf / wasm_demo …
 │     └─ [full only] full_build_and_test (cargo update + nightly), miri, win32_build(cargo xwin on ubuntu),
 │                    linuxkms_build_matrix, ios, yocto, zephyr, qa-tree-sitter-latest
 └─ done  needs[ci, format_fix, lint_typecheck]  if: always()
          → 任一结果为 failure 或 cancelled 就 exit 1   (branch protection 只要求这个 job)

schedule_ci.yaml (3x/day) ──dispatch──▶ ci.yaml(full=true) ──fail──▶ notify_ci_failure.yaml

schedule_nightly_snapshot.yaml ──dispatch──▶ nightly_snapshot.yaml(mode)
 ├─ cross_containers ─┬─ slint-viewer-binary / slint-lsp-binary (uses slint_tool_binary.yaml)
 │                    ├─ cpp_package / python_wheels
 ├─ docs (uses build_docs.yaml)   wasm_demo / wasm
 ├─ build_vscode_lsp_{linux_windows, macos_x86_64, macos_aarch64} → macos_bundle(lipo + ./.github/actions/codesign)
 ├─ publish_artifacts (mode != test)
 ├─ prepare_release (permissions: contents: write; ncipollo/release-action)
 ├─ publish_crates_io (id-token: write; rust-lang/crates-io-auth-action = crates.io trusted publishing; publish.sh --dry-run 再正式)
 └─ trigger_package_publishing → gh workflow run upload_pypi / publish_npm_package …
```

### 1.3 静态检查
- rustfmt：在 `ci:autofix:fix` 里**直接修复并由 autofix.ci 提交**，而不是只报错。
- clippy：`scripts/run_clippy.sh` 对 root/examples/demos/tests 四个 workspace 跑 `cargo clippy --locked --all-features --workspace … -- -D warnings`，失败后继续跑完其余 workspace 再统一退出。只在 ubuntu 上跑。
- `.clippy.toml`：`type-complexity-threshold`、`too-many-arguments-threshold`，以及 `disallowed-methods`（禁用 `tokio::task::spawn_local`）。只有在 crate 内写了 `#![deny(clippy::disallowed_methods)]` 才生效。
- MSRV：在 build_and_test 矩阵里把 Windows 固定到 `rust_version: "1.95"`，macOS 用 beta，full 模式下加 nightly + `cargo update`。
- miri：只在 full 模式跑（`cargo miri test -p vtable -p const-field-offset -p i-slint-common`、`-p i-slint-core --lib`）。
- 许可证：`lint:legal:reuse`（REUSE 规范）和 `cargo xtask check_license_headers`。
- TS：biome、knip、typecheck。拼写检查用 cspell。
- **没有** cargo-deny、cargo-audit、zizmor、CodeQL。action 大多用 tag 固定（`@v7`），只有 `autofix-ci/action` 按 SHA 固定。

### 1.4 文档
`build_docs.yaml`（workflow_call）：`cargo doc --no-deps --all-features`，`RUSTDOCFLAGS: -D warnings …`，另外构建 doxygen 和 astro。PR 只在 `docs_build` 分组变更时调用，nightly 每次都跑。

### 1.5 缓存与成本
- `.github/actions/setup-rust`（composite）：`dtolnay/rust-toolchain` + `Swatinem/rust-cache@v2`，**`save_if` 默认为 `github.ref == 'refs/heads/master'`**，只有 master 写缓存，PR 只读。
- 在 Windows 上：`RUSTFLAGS=-C debuginfo=0`、`git core.longpaths`、把 `CARGO_HOME` 移到短路径。全局设置 `CARGO_INCREMENTAL=false`、`CARGO_PROFILE_DEV_DEBUG=0`。
- 路径分组 `.github/ci_path_filters.yaml` 用 YAML 锚点组合（`ci_config`、`compiler`、`internal`…）。改 `Cargo.lock` 算作 `ci_config` 级别变更，会触发全部 job。
- 全量测试（cargo update、nightly、各种目标）不在 PR 上跑，交给每日 3 次的 schedule。

### 1.6 Release 与签名
- `nightly_snapshot.yaml` 同时承担 nightly 和 release，靠 `mode` 区分。复用的是**构建类** reusable（tool binary、docs、wasm），**不会**重跑 ci.yaml，靠前面的 scheduled full CI 保证质量。
- macOS：`visual_editor_macos_dmg.yaml` 由 `scripts/package_macos_visual_editor.bash` 分步完成：`install-signing-material → archive-app → stage-and-sign-app → notarize-and-staple-app`，DMG 也会公证。`codesign` composite 是标准做法：临时 keychain + `security import p12` + `codesign --force -s`。
- Windows：`visual_editor_windows_msix.yaml` 打 MSIX，用 `microsoft/microsoft-store-apppublisher` 提交到 Store，由 Store 签名。
- crates.io：`rust-lang/crates-io-auth-action`（OIDC），先 `--dry-run`。
- npm：`publish_npm_package.yaml` 设 `permissions: id-token: write`。

### 1.7 安全
顶层 `permissions: contents: read`，个别 job 再提权（例如 `deployments: write`）。`pull_request_target` 只读元数据，不 checkout PR 代码。schedule/dispatch 判断 `github.repository == 'slint-ui/slint'`，避免 fork 里也跑起来。

---

## 2. langchain-ai/langgraph（Python monorepo）+ langchain-ai/langgraphjs（TS monorepo）

> 注：`libs/sdk-js` 在 langgraph 仓库里只剩一个 README，JS 已经迁到 `langgraphjs`，所以下面合在一起讲。

### 2.1 langgraph `ci.yml`
- `on: workflow_dispatch / push: main / pull_request`，`permissions: contents: read`，`concurrency: ${{workflow}}-${{ref}}` cancel。
```
changes [dorny/paths-filter] → outputs python/deps/sdk_py  (dispatch 时用 `|| 'true'` 默认全跑)
 ├─ lint   matrix working-directory × 8  → uses ./_lint.yml  {working-directory}
 ├─ test   matrix working-directory × 7  → uses ./_test.yml  (内部 matrix python 3.10–3.14)
 ├─ test-langgraph → uses ./_test_langgraph.yml  (矩阵不同，单独抽出)
 ├─ check-sdk-methods / check-schema (重新生成 schema 后 diff，不一致就失败)
 ├─ integration-test → uses ./_integration_test.yml
 ├─ sdk-py-integration-test → uses ./_sdk_integration_test.yml
 └─ ci_success  needs[全部]  if: always()
      EXIT_CODE = !contains(needs.*.result,'failure') && !contains(...,'cancelled') ? 0 : 1
```
- `_lint.yml`（workflow_call，输入 `working-directory`）：Python 版本只跑一个（注释说明 min/max 之外几乎不会多出 lint 问题，还能省并发数）。job 内部再用 `Ana06/get-changed-files` 过滤 `${dir}/**`，**包没变就跳过所有步骤**，job 结果仍是 success，汇总 job 不受影响。命令统一走包内的 `make lint_package` / `make lint_tests`。
- `pr_lint.yml`：`amannn/action-semantic-pull-request` 检查 PR 标题，`permissions: pull-requests: read`。
- 所有第三方 action 都按 SHA 固定，并在注释里写版本号（例如 `actions/checkout@3d3c42e…# v7.0.1`）。

### 2.2 langgraph `release.yml`（复用方式）
`on: workflow_dispatch(working-directory)`，每个包单独发布：
```
build (uv build, upload dist) ─▶ release-notes ─▶ pre-release-checks
   (安装刚构建的 wheel 后跑 make test，测的是产物本身，不是源码)
      ─▶ test-pypi-publish  uses ./_test_release.yml  (permissions id-token: write) → TestPyPI
      ─▶ publish  (permissions: id-token: write only; pypa/gh-action-pypi-publish = trusted publishing)
      ─▶ mark-release (permissions: contents: write; ncipollo/release-action)
```
要点：release **不重跑** `_lint`/`_test`，而是对构建产物重跑测试。build job 没有任何写权限，注释原文是 *"a malicious build step (e.g. via a compromised dependency) could get access to our GitHub or PyPI credentials"*。

### 2.3 langgraphjs
- `ci.yml`：`lint`（oxlint）、`format`（oxfmt --check）、`build`（turbo，产物打成 tarball 上传，再用 `pkg-pr-new publish` 发 PR 预览包）→ `test` needs[lint, format, build]：ubuntu 跑 node 22/24/26，windows 和 macos **只跑一个 node 版本**。`browser-tests` 和 `environment-tests` 通过 `uses: ./test-browser.yml`、`./test-exports.yml` 复用，传入 `artifact-name`，**复用 build 产物，不重新构建**。
- `test-exports.yml`：在 docker 里用 esm/cjs/esbuild/vite/tsc/cloudflare/vercel 分别 import 产物，验证 exports map。这是 "are-the-types-wrong" 的自建版本。
- `release.yml`：`on: push main`，`changesets/action` 自动开版本 PR 或发布，`NPM_CONFIG_PROVENANCE: true`，`id-token: write`。

---

## 3. tauri-apps/tauri（Rust + JS，桌面框架）

### 3.1 文件与触发：拆成许多小 workflow，各自带路径过滤
| 文件 | 触发 | 内容 |
|---|---|---|
| `fmt.yml` | PR | `cargo fmt --all -- --check`、prettier `pnpm format:check`、taplo `fmt --check` |
| `lint-rust.yml` | push dev；PR 改了 `crates/**`、`Cargo.*` | clippy 矩阵（windows/linux/macos/ios/android 目标）：`clippy --all-targets --all-features -- -D warnings`，带 `Swatinem/rust-cache` |
| `lint-js.yml` | PR 改了 `packages/**` | eslint；`pnpm build && pnpm ts:check` |
| `test-core.yml` | push dev；PR（路径包含/排除列表） | 矩阵：OS×target，toolchain **固定 1.90（MSRV）**，移动端用 cross |
| `test-cli-rs.yml` | 同上 | `dtolnay/rust-toolchain@1.90` |
| `audit.yml` | `schedule 0 0 * * *` / dispatch；PR 和 push 仅在 lock 或 manifest 变更时 | `rustsec/audit-check@v2`（发现漏洞会开 issue，所以需要 `issues: write`）+ `pnpm audit` |
| `supply-chain.yml` | 每日；push dev 且 lock 变更 | `cargo vet suggest`（`cargo vet --locked` 被注释掉了，说明还没强制） |
| `udeps.yml` | push dev | changes → setup（动态生成 crate 矩阵）→ `cargo udeps`（nightly） |
| `check-license-header.yml` | PR | 只检查**新增文件**的 license 头（paths-filter 的 `added_files`） |
| `covector-version-or-publish.yml` | push dev | 发布编排，见下 |

所有 workflow 都设 `concurrency: ${{workflow}}-${{ref}}` cancel 和 `permissions: contents: read`。action 用 tag 固定（`@v7`），不是 SHA。

### 3.2 发布编排（发布前重跑检查）
```
push dev ─▶ check-publish (.changes/*.md 存在 → publish=true)
   ├─ audit (cargo audit + pnpm audit)
   ├─ run-integration-tests  matrix ubuntu/macos/windows
   ├─ test-api-e2e         uses ./test-api-e2e.yml          (与 PR 共用的 reusable)
   ├─ test-api-e2e-mobile  uses ./test-api-e2e-mobile.yml
   └─ version-or-publish  needs[以上] if: !cancelled() && !failure()
        covector → 开版本 PR 或 publish；随后 repository_dispatch / gh workflow run
            ├─▶ publish-cli-js.yml : build(matrix 10+ targets, napi) → test bindings(mac/win/linux×node22/24) → npm publish (id-token: write, NODE_AUTH_TOKEN='')
            └─▶ publish-cli-rs.yml : build matrix → upload (softprops/action-gh-release)
```

### 3.3 桌面 App 签名（tauri-apps/tauri-action 官方示例）
`examples/publish-to-auto-release-universal-macos-app-with-signing-certificate.yml`：
- 临时 keychain：`security create-keychain`、`set-keychain-settings -t 3600`、`import certificate.p12 -T /usr/bin/codesign`、`set-key-partition-list`，再用 `find-identity` 取得 `Developer ID Application` 身份。
- `tauri-action` 环境变量：`APPLE_ID`、`APPLE_PASSWORD`（app 专用密码）、`APPLE_TEAM_ID`、`APPLE_CERTIFICATE`、`APPLE_SIGNING_IDENTITY`（由它完成公证），`releaseDraft: true`。
- job 级 `permissions: contents: write`。

---

## 4. denoland/deno（Rust + TS，二进制分发）

### 4.1 workflow 由 TS 生成
- `ci.ts` → `ci.generated.yml`（工具是 `jsr:@david/gagen`）。文件尾部写 `# gagen:pin actions/checkout@v6 = de0fac2e…`，**生成时把 tag 自动换成 SHA**。`tools/lint.js` 里的 `ensureWorkflowYmlsUpToDate()` 会检查生成物有没有更新。
- `ci.generated.yml`：`on: push main + tags '*' / pull_request(opened, reopened, synchronize, ready_for_review)`。`concurrency` 按 head_ref 分组，带 `ci-test-flaky` 标签时改按 run_id 分组（不取消）。

### 4.2 Job 图与成本控制
```
pre_build [ubuntu]  draft PR 且提交信息不含 [ci] → skip_build；tools/check_docs_only_changes.js → docs_only
 ├─ lint  matrix(linux/mac/windows)  format.js --check (linux only), jsdoc_checker, lint.js
 │        lint.js: dlint/JS lint, copyright 头, clippy(-D warnings + --deny clippy::print_stdout/print_stderr/
 │                 unused_async/large_futures/allow_attributes_without_reason),
 │                 ensureDisallowedMethodsEnforced(检查每个 crate 的 clippy.toml 是否包含规定的禁用方法清单),
 │                 ensureWorkflowYmlsUpToDate
 ├─ build-{debug,release}-{linux,macos,windows}-{x86_64,aarch64}  (environment: 'build' 仅 main/tag)
 │     PR 且无 ci-full 标签：mac/windows release 构建 runner 改成 ubuntu-24.04，并 skip
 │     → test-*-*  分片(shard_index/shard_total)
 ├─ deno-core-test / deno-core-miri / bench(仅打了 ci-bench 标签或 main)
 ├─ ci-status  needs[全部] if: always()  → "Ensure CI success"
 └─ publish-canary  needs[所有 build] if: denoland/deno && main
```
另外：缓存 key 带 `cacheVersion` 常量，要清缓存时改这个数字；release 才跑的 startup-order 二次链接等耗时步骤只在 `isMainOrTag || ci-full` 时做。

### 4.3 签名
- macOS：在 macOS 构建 job 中用 `rcodesign sign --code-signature-flags=runtime --p12-file=<(… base64 -d) --entitlements-xml-file=cli/entitlements.plist`，只在 main 或 tag 上执行。
- Windows：先 `azure/login@v2`（OIDC，注释提醒 *Azure OIDC credentials are only valid for 5 minutes, so authentication must be done right before signing*），再 `Azure/artifact-signing-action`（Trusted Signing，参数含 endpoint、account、certificate-profile、RFC3161 时间戳），最后 `signtool verify /pa /v` 校验。
- Release 流程：`start_release` → `version_bump` → `create_prerelease_tag` → `promote_to_release`（`environment: build`，`id-token: write`，把 canary 产物重新标记版本并重签）。
- npm：`npm_publish.generated.yml`：build → test（**先发布到本地 Verdaccio，再 `npm install -g` 冒烟测试**）→ publish（`permissions: id-token: write`，带 `dry_run` 输入）。
- 安全：`pr.generated.yml` 顶部注释提醒 PR 上下文中的 GITHUB_TOKEN 有写权限。PR 标题检查只是 `deno run` 远程脚本，不 checkout PR 代码。

---

## 5. BurntSushi/ripgrep（Rust CLI，release 构建的经典写法）

- `ci.yml`：`on: pull_request / push master / schedule '00 01 * * *'`，`permissions: contents: read`。
  - `test` 矩阵：`pinned 1.96.0`（MSRV）、stable、beta、nightly、musl、x86、多个 cross 目标（aarch64/powerpc/s390x…）、macOS、Windows msvc/gnu。
  - `rustfmt`（`cargo fmt --all --check`）、`docs`（`cargo doc --no-deps --document-private-items --workspace`）、`wasm`、`fuzz_testing`（只做 `cargo check`）。
  - 没有缓存、没有汇总 job、没有 clippy。
- `release.yml`：`on: push tags "[0-9]+.[0-9]+.[0-9]+"`；`permissions: contents: write, id-token: write, attestations: write`。
```
create-release : 校验 tag == Cargo.toml version → gh release create $V --draft --verify-tag
   ├─ build-release (matrix 10+ targets, cross v0.2.5 固定版本, --profile release-lto, strip,
   │                 打包 zip/tar.gz + sha256 + actions/attest-build-provenance(SHA 固定) → gh release upload)
   └─ build-release-deb (cargo deb)
draft 由人工检查后再 publish
```

---

## 6. tokio-rs/tokio（Rust 库，静态检查最完整）

- `ci.yml`：`on: push (master, tokio-*.x) / pull_request`，`permissions: contents: read`，concurrency cancel。全局 `env: RUSTFLAGS: -Dwarnings`，并把 toolchain 都固定成变量：`rust_clippy: '1.88'`、`rust_min: '1.85'`、`rust_nightly: nightly-2025-10-12`、`rust_miri_nightly`……
```
fmt ─┐
clippy (固定 1.88, --workspace --tests --no-deps, 分 stable 特性 / all-features+unstable 两轮) ─┤
docs (windows+ubuntu, RUSTDOCFLAGS: --cfg docsrs -Dwarnings) ─┤
minrust (1.85 cargo check) ─┘
   └─▶ basics (run: exit 0)   ← 便宜检查的闸门
         └─▶ ~40 jobs: test-tokio-full ×3 OS, all-features, panic-abort, miri-lib/test/doc, asan, valgrind,
                       semver (obi1kenobi/cargo-semver-checks-action), minimal-versions (cargo update -Z minimal-versions),
                       cross-check, wasm32-*, check-external-types, check-spelling (cargo-spellcheck + 字典排序检查),
                       loom-compile, freebsd, io-uring kernel 矩阵 (uses ./uring-kernel-version-test.yml)
```
- `audit.yml`：`push master paths **/Cargo.toml` + `schedule 0 2 * * *` → `EmbarkStudios/cargo-deny-action@v2`（job 权限 `checks: write, issues: write`）。
- `pr-audit.yml`：PR 或 push 改了 `**/Cargo.toml` 才跑 cargo-deny。**不在每个 PR 上都跑 advisories**，避免新公布的漏洞让无关 PR 变红；每日定时任务负责发现新漏洞。
- `deny.toml`：`[licenses] allow = ["MIT","Apache-2.0"]` + `exceptions = [{allow=["Unicode-3.0"], crate="unicode-ident"}]`；`[bans] wildcards="deny"`；`[sources] unknown-registry/unknown-git = "deny"`。

---

## 7. astral-sh/uv（Rust，编排和发布安全做得最好）

### 7.1 `ci.yml` 只做编排
`on: push main / pull_request / workflow_dispatch`，`permissions: contents: read`，concurrency cancel。
```
plan (uses ./plan.yml)  → 按改动文件 + PR 标签算出：test-code, test-macos, run-checks, save-rust-cache,
                          build-release-binaries, check-crates-policies …
                          标签: test:skip test:macos test:integration test:extended build:release build:skip …
 ├─ check-fmt / check-lint / check-docs / check-zizmor / check-lock / check-publish / check-release
 │  check-generated-files / check-crates-policies   (都是 uses ./check-*.yml)
 ├─ test (uses ./test.yml, with test-macos)   ← macOS 测试默认关闭，按 plan 输出或 test:macos 标签开启
 ├─ build-dev-binaries ─▶ test-smoke / test-integration / test-system / test-ecosystem (复用 dev 二进制)
 ├─ build-release-artifacts (uses ./build-release-binaries.yml, 仅 build:release 标签)  ← 与 release.yml 是同一个文件
 └─ required-checks-passed  if: always() && ref != main
      jq 遍历 needs，结果不是 success/skipped 就失败；CONDITIONAL_JOBS 表声明哪些 job 允许按条件跳过
```
- `check-lint.yml`：ruff、ty、shellcheck、validate-pyproject、`clippy-ubuntu` 和 `clippy-windows`（`cargo clippy --workspace --all-targets --all-features --locked`）、**cargo-deny**（`check licenses`、`check sources`、`scripts/check-cargo-bans.py`、`check bans`）、**cargo-shear --deny-warnings**（未使用依赖）、**typos**、hawk（`-D warnings`）。
- lint 配置写在 `Cargo.toml [workspace.lints]`：`rust.unsafe_code = "warn"`、`unreachable_pub = "warn"`；`clippy.pedantic = {level="warn", priority=-2}` 外加一组 allow，以及 `print_stdout`/`print_stderr`/`dbg_macro`/`exit`/`get_unwrap`… 为 warn。warn 靠 CI 环境升级为错误。
- `.cargo/deny.toml`：`[bans] wildcards="deny"`，**`[bans.build] allow-build-scripts = [...]`（允许带 build.rs 的依赖白名单）**。
- `check-zizmor.yml`：`zizmorcore/zizmor-action`，`security-events: write`，结果上传到 code scanning。
- `check-crates-policies.yml`：顶层 `permissions: {}`，job 级 `environment: automations`、`id-token: write`，按外部策略仓库审核新增 crate。
- 缓存：输入 `save-rust-cache`，由 plan 决定是否写缓存（主要是 main）。

### 7.2 `release.yml`（复用 CI 的构建，签名独立）
`on: workflow_dispatch(dry-run)`，**顶层 `permissions: {}`**，`ACTIONS_CACHE_MODE: none`（release 不使用缓存，防止缓存投毒）。
```
release-gate (environment: release-gate → 人工批准)
plan (cargo-dist plan)
 ├─ build-release-artifacts  uses ./build-release-binaries.yml   (与 CI 的 build:release 是同一个 reusable)
 ├─ build-docker            uses ./build-docker.yml
 └─ assemble-release-artifacts  needs[plan, build, release-gate]  uses ./sign-release-binaries.yml
        prepare ─┬─ sign-macos   [ubuntu runner!] environment: release, id-token: write
                 │     astral-sh/github-actions/sign-macos: rcodesign + Azure Key Vault PKCS#11 + Apple 公证
                 ├─ sign-windows [windows-2025]   environment: release, azure/artifact-signing-action
                 └─ assemble → verify-macos / verify-windows (在真实 OS 上验证签名后的产物)
 → generate-local-dist-manifest → build-global-dist-artifacts → finalize-dist-manifest
 → publish-pypi / publish-crates / publish-docker (attestations: write)
 → publish-github (actions/attest-build-provenance) → publish-docs / publish-versions / publish-mirror
```
每个 job 单独声明最小权限，所有 action 都按 SHA 固定。

---

## 8. vitejs/vite（TS）

- `ci.yml`：`on: push (main, release/*, feat/*, fix/*, perf/*, v*) / pull_request / dispatch`，**顶层 `permissions: {}`**，concurrency cancel。
```
changed [ubuntu-slim] tj-actions/changed-files → should_skip(只改文档时)
 ├─ test needs changed  matrix: ubuntu × node 20/22/24/26 + macos@24 + windows@24 (各只跑 1 个)
 │     build → test-unit → test-serve → test-serve-bundled → test-build
 ├─ test-passed (if !cancelled && !failure) / test-failed   ← 用于 branch protection
 └─ lint [ubuntu]: pnpm build, oxlint, oxfmt + git diff --exit-code, tsc typecheck (tsc -p scripts && pnpm -r typecheck), test-docs
```
- `zizmor.yml`：PR / push main（`.github/workflows/**`），`security-events: write`。
- `semgrep.yml`：每周跑，结果用 SARIF 上传到 code scanning。
- `publish.yml`：`on: push main / v*`，`detect-release`（从提交信息判断是否发布）→ `publish-release`：`environment: Release`、`permissions: contents: write, id-token: write`、**`npm i -g npm@^11.5.2`（npm trusted publishing 要求 ≥ 11.5）**、`pnpm run ci-publish`（`@vitejs/release-scripts`）。另外 `yq '.allowBuilds[]=false'`：发布时禁止依赖执行 build 脚本。
- `preview-release.yml`：打了 `trigger: preview` 标签的 PR 用 `pkg-pr-new` 发预览包。

---

## 9. 各项目横向对比

| 项 | slint | langgraph(js) | tauri | deno | ripgrep | tokio | uv | vite |
|---|---|---|---|---|---|---|---|---|
| 入口 + reusable | ✔ autofix→ci→*_reusable | ✔ _lint/_test | 部分 | 生成器 | ✗ | 少量 | ✔✔ | ✗ |
| 汇总 job | done | ci_success | ✗ | ci-status | ✗ | basics(前置闸门) | required-checks-passed | test-passed |
| 路径过滤 | paths-filter 分组 | paths-filter + 包内 changed-files | on.paths | docs-only 脚本 | ✗ | on.paths(audit) | plan.yml | changed-files |
| 昂贵 OS 策略 | full 只在定时任务 | mac/win 只跑一个 node 版本 | 全跑 | PR 无标签就跳过 | 全跑 | 全跑 | test:macos 标签 | mac/win 只跑一个版本 |
| clippy -D warnings | ✔(ubuntu) | – | ✔(5 target) | ✔ + 额外 deny | ✗ | ✔(RUSTFLAGS) | ✔(ubuntu+win) | – |
| cargo-deny/audit | ✗ | – | audit + vet | – | ✗ | deny(每日 + 改动时) | deny licenses/sources/bans | – |
| MSRV | 1.95 | – | 1.90 | rust-toolchain | 1.96.0 | 1.85 | rust-toolchain.toml | – |
| 未使用依赖 | knip(TS) | – | udeps | – | – | – | cargo-shear | – |
| 拼写 | cspell | – | – | – | – | cargo-spellcheck | typos | – |
| workflow 审计 | ✗ | – | ✗ | 生成物校验 | ✗ | ✗ | zizmor | zizmor + semgrep |
| SHA 固定 | 很少 | ✔ | ✗ | ✔(自动) | 部分 | ✗ | ✔ | ✔ |
| 签名 | codesign + notarize(mac 脚本)、MSIX Store | – | tauri-action | rcodesign + Azure TS | – | – | Linux 上 rcodesign + 公证；Azure | – |
| 来源证明 | crates OIDC | npm provenance | npm OIDC | npm OIDC | attest-build-provenance | – | attest + OIDC | npm OIDC |
| draft release | ✔ | – | releaseDraft | ✔ | ✔ --draft --verify-tag | – | dist | – |

---

## 10. 最佳实践模式与 Vestra Player 推荐方案

### 10.0 背景约束（来自仓库现状）
- Repo A（`vestra-player`，私有）：`src/host` 是 Rust 包 `vestra-host`（GPL-3.0-or-later，edition 2024），`[workspace] members = ["unsafe/pip","unsafe/render","unsafe/win32"]`，host 已有 `[lints.rust] unsafe_code = "forbid"`。依赖里有 libmpv（`rsmpv`）、Slint 1.18、objc2（macOS）、windows-registry 等。`src/package.json` 已有 `check`(tsc)、`format:check`(prettier)、`boundaries`、`portability`、`test:sdk`、`plugins:build`、`sdk:sync-slint`。共 18 个插件，位于 `src/plugins/*`。
- 目前还没有 `.github/workflows`。
- 计费：私有仓库 Linux 1x、Windows 2x、macOS 10x。主力开发机是 macOS。
- 平台：现在是 macOS + Windows，以后加 Linux/Android/iOS。host **无法在 Linux 上编译**（大量 `cfg(target_os)` 平台代码），所以 clippy 和测试必须在目标 OS 上跑，或交叉检查。

### 10.1 通用原则（从上面项目归纳）
1. **入口 workflow 只做编排**（uv/slint/langgraph）：`ci.yml` 和 `release.yml` 只写 `uses: ./.github/workflows/_xxx.yml`，检查逻辑只写一份。
2. **静态检查在 ubuntu 上做闸门**（tokio `basics`、slint `needs[format_fix, lint_typecheck]`）：fmt、cargo-deny、TS、unsafe 策略这类检查不通过，就不启动 Windows 和 macOS 构建。
3. **plan job 统一决定跑什么**（uv `plan.yml`）：根据路径、分支、标签输出 `run-macos`、`run-windows` 等。
4. **macOS 默认不跑，按条件开启**（deno `ci-full`、uv `test:macos`、slint 定时 full）。
5. **只设一个 required check**：`ci-ok`，`if: always()`，任一 needs 为 failure 或 cancelled 就失败。被跳过的 job 视为通过（slint `done`、uv `required-checks-passed`）。
6. **只有 main 写缓存**（slint `save_if`、uv `save-rust-cache`）。release 不使用缓存（uv `ACTIONS_CACHE_MODE: none`）。
7. **构建和发布/签名分开**（langgraph、uv）。签名 secrets 放在 `environment: release`，限定只允许 tag 部署，并设人工审批（uv `release-gate`）。
8. **顶层 `permissions: {}` 或 `contents: read`，在 job 级再提权**；所有 action 按 SHA 固定（uv/vite/langgraph），交给 Renovate/Dependabot 维护；用 zizmor 扫描 workflow（uv/vite）。
9. **不使用 `pull_request_target` checkout PR 代码**（slint `pr_checks.yaml` 注释）。
10. **生成物必须和源码一致**：先重新生成，再 `git diff --exit-code`（langgraph check-schema、deno ensureWorkflowYmlsUpToDate、vite `format && git diff --exit-code`），对应我们的 SDK 副本检查。

### 10.2 Repo A（私有：Rust host + Slint + 18 个 TS 插件）

#### 文件布局
```
.github/
  actions/
    setup-rust/action.yml      # composite: rust-toolchain.toml + Swatinem/rust-cache(save-if: main) + Windows 短路径/longpaths
    setup-node/action.yml      # composite: setup-node + npm ci (cache)
  workflows/
    ci.yml                     # 入口: push(任意分支) / pull_request / merge_group / workflow_dispatch(full)
    nightly.yml                # schedule: main 有新提交才跑 → uses _audit + _build-test(full) + _package(sign=false)
    release.yml                # tag v* → 复用 _static + _build-test → _package(sign) → draft release
    _plan.yml                  # workflow_call: paths/labels/ref → outputs
    _static.yml                # workflow_call: 全部 ubuntu 静态检查
    _build-test.yml            # workflow_call(os, toolchain, full): clippy + test + doc，在目标 OS 上
    _package.yml               # workflow_call(os, sign): release build + bundle + 签名 + 产物
    _audit.yml                 # workflow_call: cargo-deny advisories (允许单独定时跑)
  CODEOWNERS                   # src/host/unsafe/** 需要额外审查
deny.toml / clippy.toml / _typos.toml / rust-toolchain.toml / about.toml(cargo-about)
src/scripts/check-unsafe-policy.mjs (或 xtask)
```

#### ci.yml job 图
```
ci.yml  on: push{branches:['**'], tags-ignore:['**']} / pull_request / merge_group / workflow_dispatch{full}
        permissions: {}   concurrency: ci-${{ github.event.pull_request.number || github.ref }} cancel-in-progress
        (单人私有仓库可以只用 push；需要 PR 页面状态时再加 pull_request，并用 concurrency 去重)

plan  [ubuntu, 1min]  uses _plan.yml
  outputs: rust(host/**, Cargo.*), ts(plugins/runtime/scripts/package*), mac_only(platform/macos*, unsafe/pip, unsafe/render),
           win_only(platform/windows*, unsafe/win32), run_macos, run_windows, full
  run_macos = ref==main || event==merge_group || label 'ci:macos' || full || (mac_only && 是 PR)
  run_windows = rust 变更 (任意分支)          ← 本地开发机覆盖不到 Windows，所以默认跑
 │
 ├─ static  [ubuntu, ~5–8min]  uses _static.yml          ← 闸门，1x 计费
 │     Rust: cargo fmt --check · taplo fmt --check · typos
 │           cargo deny check licenses bans sources (多目标 [graph].targets, 见下)
 │           cargo-shear (未使用依赖, 不需要编译)
 │           check-unsafe-policy (见 10.4)
 │           cargo xwin clippy --target x86_64-pc-windows-msvc -- -D warnings   ← 在 Linux 上交叉检查 Windows (slint win32_build 用过 cargo xwin)【可选，可行性需验证】
 │     TS:   npm ci · npm run check(tsc) · format:check · boundaries · portability · test:sdk · plugins:build
 │           sdk copy 检查: npm run sdk:sync-slint && git diff --exit-code
 │           SDK 版本副本 == npm 上 @vestra-player/sdk@<lock 版本> (npm pack 后 diff)
 │     CI:   actionlint · zizmor (只在 .github/** 变更时, uv/vite)
 │
 ├─ windows [windows-2022, ~15min, 2x]  needs[plan, static] if run_windows   uses _build-test.yml{os}
 │     cargo clippy --locked --all-targets --all-features -- -D warnings
 │     cargo test --locked  (含 unsafe/* workspace 成员)
 │     cargo doc --no-deps --document-private-items  (RUSTDOCFLAGS=-D warnings)  ← 只在 Windows 跑，省 macOS 时长
 │
 ├─ macos   [macos-15 arm64, ~15min, 10x]  needs[plan, static] if run_macos   uses _build-test.yml{os}
 │     clippy -D warnings · cargo test   (doc 不重复跑)
 │
 └─ ci-ok   needs[plan, static, windows, macos]  if: always()
       contains(needs.*.result,'failure'||'cancelled') → exit 1     ← branch protection 唯一 required check
```
补充：
- **merge 前保证 macOS 跑过**：PR 合并或 merge_group 时 `run_macos=true`；feature 分支的普通 push 依赖本地 macOS 开发机上跑的 `npm run ci:local`。在 `package.json` 加一个脚本，执行和 `_static` + `_build-test` 相同的命令，保证本地和 CI 一致。
- 全局 env（slint/tauri）：`CARGO_INCREMENTAL=0`、`CARGO_PROFILE_DEV_DEBUG=0`、`CARGO_TERM_COLOR=always`；每个 job 都设 `timeout-minutes`。
- 缓存：`Swatinem/rust-cache` 设 `save-if: ${{ github.ref == 'refs/heads/main' }}`，key 区分 OS。仓库缓存上限 10GB，macOS 和 Windows 各一份 target 很容易超，所以只让 main 写入。

#### nightly.yml（schedule，01:xx UTC；参考 slint `schedule_ci` 写法：main 的 tip 有变化才跑全量）
```
_audit         cargo deny check advisories  (每天都跑，因为 RustSec 会新增漏洞；PR 只在 Cargo.lock 变更时跑，见 tokio pr-audit)
_build-test    macos + windows, toolchain=stable 最新 (rust-toolchain.toml 固定版本以外的"下个版本预警"，参考 slint beta / ripgrep beta)
_build-test    cargo update 后测试 (参考 slint full_build_and_test)，失败只通知不阻塞
_package       sign=false，产出未签名的 nightly 包供 GUI 测试
→ 失败时用 workflow_run 或 issue 通知 (slint notify_ci_failure.yaml)
```

#### release.yml（复用同一套检查，签名独立）
```
on: push tags 'v*'  (+ workflow_dispatch{dry_run})   permissions: {}   不使用缓存
verify   [ubuntu] tag == src/host/Cargo.toml version (ripgrep create-release)
static   uses _static.yml                       ┐ 与 CI 是同一个文件
windows  uses _build-test.yml{windows}          │
macos    uses _build-test.yml{macos}            ┘
package-mac [macos-15]   needs[...]  uses _package.yml{os: macos, sign: false}
     cargo build --release --locked → .app bundle (含 libmpv dylib、THIRD_PARTY_LICENSES、插件) → upload unsigned
package-win [windows]    uses _package.yml{os: windows}
     cargo build --release → installer(MSI/MSIX/NSIS) → 签名:
       environment: release · id-token: write · azure/login(OIDC) → Azure/artifact-signing-action → signtool verify /pa
       (deno ci.ts / uv sign-windows)
sign-mac [ubuntu!]  environment: release · needs package-mac          ← 省钱的关键 (uv sign-macos)
     rcodesign sign --code-signature-flags=runtime --entitlements … (.app 内每个 dylib/可执行文件)
     rcodesign notary-submit --staple (App Store Connect API key)  → 公证等待在 1x runner 上
     → DMG 签名 + 公证
     备选: 直接在 macOS job 里 codesign + xcrun notarytool (slint/tauri-action)，更简单但要按 10 倍计费等待
verify-mac [macos-15, 短] codesign --verify --deep --strict · spctl -a -vv (uv verify-macos 思路)
checksums  sha256sum → SHA256SUMS
attest     actions/attest-build-provenance   ⚠ 私有仓库需要 GitHub Enterprise Cloud 才能用 artifact attestations【推断，按 GitHub 文档】；
           不可用时就用签名 + SHA256SUMS
release    gh release create $TAG --draft --verify-tag  (ripgrep)  → 人工检查后 publish
           (或单独加一个 publish job，用 environment: release-publish 人工批准，参考 uv release-gate)
```
Environment 配置：`release` 只允许 `v*` tag 部署，开启 required reviewers（单人项目也可以作为确认步骤），里面存 Apple 证书或 ASC API key、Azure client/tenant/subscription id。日常 CI 拿不到这些 secrets。

#### Rust 检查配置
- **rustfmt** + **taplo**（tauri `fmt.yml`）。
- **clippy**：`cargo clippy --locked --all-targets --all-features -- -D warnings`（tauri/slint）。lint 写进 `Cargo.toml` 的 `[workspace.lints]`，**不要直接全开 pedantic**。参考 uv 的方法，先设 `pedantic = {level="warn", priority=-1}` 再逐条 allow，或者只挑这几条：`dbg_macro`、`print_stdout`、`print_stderr`（host 有日志系统时）、`allow_attributes_without_reason`（deno）、`undocumented_unsafe_blocks`（unsafe crate）、`unwrap_used`（可选）。
- **clippy.toml**：`disallowed-methods`（slint/deno），可以拿来把跨平台规则落到编译期，例如在非 platform 模块里禁用 `std::process::Command`；`check-portability.mjs` 已经覆盖一部分。
- **MSRV**：Vestra 是应用不是库，**不需要 MSRV 矩阵**。用 `rust-toolchain.toml` 固定版本（uv、deno），nightly 用最新 stable 做预警。
- **cargo-deny**（必须有，uv 和 tokio 的配置都能参考）：
```toml
[graph]
targets = ["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc"]   # 以后加 linux/android/ios
all-features = true
[licenses]
# host 是 GPL-3.0-or-later：只放行与 GPLv3 兼容的许可证
allow = ["MIT","Apache-2.0","Apache-2.0 WITH LLVM-exception","BSD-2-Clause","BSD-3-Clause","ISC","Zlib",
         "Unicode-3.0","MPL-2.0","BSL-1.0","CC0-1.0","0BSD","LGPL-2.1-or-later","LGPL-3.0-or-later",
         "GPL-3.0-or-later","OFL-1.1","Unlicense"]
confidence-threshold = 0.9
# 对许可证不清楚的 crate 用 [[licenses.clarify]] 显式声明
# 不在 allow 里的（GPL-2.0-only、AGPL、SSPL、商业许可）会直接失败
[bans]
wildcards = "deny"
multiple-versions = "warn"
# 可选: [bans.build] allow-build-scripts = [...]  (uv): 新增带 build.rs 的依赖时必须审核
[sources]
unknown-registry = "deny"
unknown-git = "deny"
[advisories]
yanked = "deny"
```
  - 许可证随包附带：release 打包时用 **cargo-about**（`about.toml` 里的 accepted 列表和 deny.toml 保持一致）生成 `THIRD_PARTY_LICENSES.html/txt` 放进 `.app` 和安装包【推断：被调研项目都没在 CI 里生成第三方许可证文件；slint 用的是 REUSE + license 头检查】。CI 在 `_static` 里运行 `cargo about generate`，**只检查能否成功生成**，防止新依赖缺少许可证文本；正式生成放在 `_package`。
  - 非 Cargo 依赖（libmpv 及其 ffmpeg 等动态库、字体、npm 运行时依赖）：维护一份 `third_party/manifest.toml`，写明名称、版本、许可证和文本路径。`check-third-party.mjs` 校验每一项都有文本，并且和 bundle 里实际的 dylib/dll 列表一致。
- **cargo-audit 和 cargo-deny 二选一**：两者都读 RustSec 数据库，用 `cargo deny check advisories` 就够了。PR 只在 `Cargo.lock` 变更时跑（tokio `pr-audit.yml`），每日定时全量跑（tokio `audit.yml`、tauri `audit.yml`）。
- **未使用依赖**：cargo-shear（uv，不需要编译，速度快）。cargo-udeps 需要 nightly 并且要编译（tauri），不推荐。
- **typos**（uv）。**actionlint + zizmor**（uv/vite）。
- **miri**：unsafe crate 基本都是 FFI（AVKit、OpenGL/IOSurface、Win32），miri 跑不了 FFI，**不加**。可以改为给 unsafe crate 补上纯 Rust 部分的单元测试。

### 10.3 成本估算与策略（私有仓库）
假设冷缓存下 Windows 构建+测试 15 分钟、macOS 15 分钟、ubuntu 静态检查 6 分钟【推断，需要实测】：

| 场景 | 跑什么 | 计费分钟 |
|---|---|---|
| feature 分支 push（只改 TS） | plan + static | ≈ 7 |
| feature 分支 push（改了 Rust） | + windows | ≈ 7 + 30 = 37 |
| PR 合并、main、merge_group | + macos | ≈ 37 + 150 = **187** |
| nightly（有新提交时） | audit + mac + win + 未签名包 | ≈ 250–350 |
| release | static + 双 OS 测试 + 打包 + macOS 仅 verify（签名和公证在 Linux 上） | ≈ 300–400；如果在 macOS 上等公证，再加 10×(5–30) |

省钱手段，按收益排序：
1. macOS 只在 main、merge、`ci:macos` 标签和 nightly 时跑（deno/uv）；本地 macOS 用 `ci:local` 兜底。
2. 先用 ubuntu 闸门，失败就不启动 Windows 和 macOS（tokio basics）。
3. 签名和公证放在 ubuntu 上（uv `sign-macos` + rcodesign），macOS runner 只做编译和短时间的验证。
4. 只有 main 写缓存，release 不使用缓存。设置 `CARGO_INCREMENTAL=0` 和 `debug=0`（slint/tauri）。
5. concurrency 取消旧的运行，所有 job 设 `timeout-minutes`。
6. 路径过滤：只改 `doc/**` 时 plan 什么都不跑（deno docs_only、vite should_skip）。
7. 以后加 Linux：Linux 是最便宜的完整构建平台，可以把 clippy 和大部分测试作为第一道编译闸门放到 Linux（slint 的 clippy 只在 ubuntu 上跑），macOS 和 Windows 只跑平台相关测试。Android/iOS 只做 `cargo check --target` 交叉检查（slint `cross_compile_checks`），完整构建放到 nightly。

### 10.4 unsafe 只允许出现在 `src/host/unsafe/*`：如何强制
现状：host 设了 `[lints.rust] unsafe_code = "forbid"`，它对该包所有 target 都生效（lib/bin/tests/build.rs）；三个 unsafe crate 是独立的包。还需要补上下面几层：

1. **改用 workspace 级 lint**：在 `[workspace.lints.rust]` 里设 `unsafe_code = "forbid"`，`[workspace.lints.clippy]` 放共用的 clippy 配置；host 写 `[lints] workspace = true`。Cargo 不允许在 `workspace = true` 之外再覆盖单条 lint，所以 **unsafe/* 三个 crate 不继承**，各自写完整的 `[lints]`：
   ```toml
   [lints.rust]
   unsafe_op_in_unsafe_fn = "deny"
   missing_docs = "warn"
   [lints.clippy]
   undocumented_unsafe_blocks = "deny"      # 每个 unsafe 块都要有 // SAFETY:
   multiple_unsafe_ops_per_block = "deny"
   missing_safety_doc = "deny"
   # + 复制 workspace 的 clippy 集合
   ```
2. **check-unsafe-policy 脚本**（参考 deno `tools/lint.js` 的 `ensureDisallowedMethodsEnforced`：遍历每个 crate，检查必须存在的配置），放在 `_static` 里，在 ubuntu 上运行：
   - 用 `cargo metadata --no-deps` 列出所有 workspace 包。路径不在 `src/host/unsafe/` 下的包，`lints.workspace` 必须是 true，或者显式写了 `unsafe_code = "forbid"`；否则失败。这样新增 crate 时忘记加也会被拦住。
   - 所有 `.rs` 文件（去掉注释和字符串后）匹配 `\bunsafe\b`，命中位置不在 `src/host/unsafe/**` 下就失败。和编译期的 forbid 互为补充，也能覆盖 `#[cfg(windows)]` 这类在当前平台不编译的代码。**这一点很关键**：在 macOS 上跑 forbid 编译检查，查不到 Windows 分支的代码。
   - 同时检查 `#[allow(unsafe_code)]`、`#![allow(unsafe_code)]` 和 `[lints] unsafe_code = "allow"` 有没有出现在不该出现的位置。
   - unsafe/* 中每个 `unsafe {` 块前面必须有 `// SAFETY:`（clippy 只检查当前编译平台，脚本可以覆盖全部平台）。
   - 白名单：unsafe crate 列表写在脚本里。新增 unsafe crate 必须同时改脚本和 CODEOWNERS，在 diff 里能直接看到。
3. **CODEOWNERS**：`src/host/unsafe/** @zysoong`，加上 branch protection 的 "Require review from Code Owners"，为以后有协作者做准备。
4. **依赖层面的 unsafe**：交给 cargo-deny（sources/bans）。如果想更严格，可以用 `[bans.build] allow-build-scripts`（uv）审核带 build.rs 的依赖；cargo-vet（tauri `supply-chain.yml`）对单人项目成本太高，不推荐。

### 10.5 TS 部分（Repo A 内的插件和运行时）
已有脚本全部放进 `_static`：`tsc --noEmit`、`prettier --check`、`boundaries`、`portability`、`test:sdk`、`plugins:build`、`sdk:sync-slint && git diff --exit-code`。
可以补充：
- **oxlint**（vite、langgraphjs 都用；速度快，几乎不用配置）或 eslint 的 typescript-eslint strict 配置。
- 18 个插件可以用 langgraph 的模式按目录建矩阵：`changes` 算出改动的插件 → `matrix: plugin` → 复用 `_plugin.yml`（tsc、单测、打包 `.vxp`、manifest schema 校验）。插件构建很快，**直接一次全跑也可以**，等变慢了再拆。
- SDK 副本检查（Repo A ↔ Repo B）：Repo A 锁定 `@vestra-player/sdk` 版本，CI 执行 `npm pack @vestra-player/sdk@$(node -p "require('./package-lock.json')…")` 后和仓库里的副本 diff，确保副本和 npm 发布的完全一致（思路同 langgraph check-schema）。

### 10.6 Repo B（公开 TS SDK，npm 发布）
```
ci.yml  on: push main / pull_request / merge_group     permissions: {}   concurrency cancel
 ├─ lint     [ubuntu] oxlint|eslint · prettier --check · tsc --noEmit · typos
 ├─ build    [ubuntu] tsc/tsup build → upload dist (langgraphjs: 测试复用 build 产物)
 │           publint · @arethetypeswrong/cli (attw --pack)  【推断：被调研项目里没看到这两个；langgraphjs 用 docker 多环境 import 测试实现同样目的】
 │           npm pack --dry-run → 检查 files 白名单 (不能带 src/test/map 以外的东西)
 │           typedoc --treatWarningsAsErrors (文档构建；main 上再部署到 Pages)
 ├─ test     needs[lint, build]  matrix: ubuntu × node LTS(20/22/24) + windows@22 + macos@22  (vite/langgraphjs: 非 Linux 只跑一个版本)
 ├─ exports  needs build: esm/cjs/types 实际 import 测试 (langgraphjs test-exports.yml)
 ├─ zizmor   (only .github/** changed)
 └─ ci-ok    aggregator
preview.yml  打了 'preview' 标签 → pkg-pr-new publish (vite/langgraphjs)，Repo A 和 Repo C 可以先装这个包试用

release.yml  on: push tags 'v*'   (或 changesets/action 自动开版本 PR，参考 langgraphjs)
 ├─ verify: tag == package.json version (ripgrep)
 ├─ ci:   uses ./ci.yml  (ci.yml 同时声明 workflow_call，发布前完整重跑)
 ├─ build: npm pack → upload tgz          (无写权限; langgraph 的 build/publish 分离)
 ├─ smoke: 安装 tgz 后跑示例插件的 tsc (deno 用 Verdaccio 本地 registry 冒烟)
 └─ publish  environment: npm (required reviewers, 只允许 v* tag)
       permissions: id-token: write, contents: write
       npm i -g npm@^11.5   (vite)  →  npm publish <tgz> (Trusted Publishing: npmjs.com 后台绑定 repo + workflow + environment，
                                       不存 NPM_TOKEN，provenance 自动附带)
       gh release create (notes 取自 CHANGELOG)
```
其他：所有 action 按 SHA 固定 + Renovate/Dependabot；`.github/workflows/**` 设 CODEOWNERS；开启 tag 保护规则；npm 包设置 "require 2FA and disallow tokens"，只允许 trusted publishing 发布。

### 10.7 Repo C（公开插件仓库，已有签名索引发布）
需要补充：
1. **PR CI 按插件建矩阵**（langgraph `ci.yml` + `_lint.yml` 模式）：`changes`（paths-filter 输出改动的 `plugins/<id>`）→ `matrix.plugin` → `uses: ./_plugin.yml`（tsc、lint、单测、打包 `.vxp`、manifest JSON schema 校验、权限声明检查、对照 SDK 版本范围的兼容性检查）→ `ci-ok` 汇总。
2. **PR 上做签名索引的 dry-run**：用临时生成的测试密钥签名，再用对应公钥验签，验证索引格式和 sha256 计算流程。**真实私钥只在 main 或 tag 的发布 job 里使用**，放在 `environment: plugin-index`，设 required reviewers 并限定分支。
3. **发布 job 与构建 job 分离**：build job 产出 `.vxp` 和 sha256，不接触任何 secret；sign-index job 只下载产物、签名、上传。理由同 langgraph `release.yml` 的注释。
4. **来源证明**：Repo C 是公开仓库，可以免费用 `actions/attest-build-provenance` 给每个 `.vxp` 生成证明（ripgrep/uv）。客户端以后可以选择用 `gh attestation verify` 的同类逻辑校验。
5. **安全**：严禁用 `pull_request_target` checkout 第三方插件 PR 的代码（slint `pr_checks.yaml`），fork PR 只跑无 secret 的 `pull_request`；`permissions: {}`；zizmor；actions 按 SHA 固定。社区插件 PR 可以加一个 `pull_request_target` 的只读元数据检查（贡献协议、manifest 中声明的权限变化），做法同 slint `pr_checks.yaml`。
6. **定时**：每周用最新发布的 SDK 重新构建全部插件，提前发现 SDK 破坏性变更（与 slint 定时 full CI 的思路相同）。

### 10.8 推荐的整体关系图
```
                ┌──────────── Repo B (public SDK) ────────────┐
                │ ci.yml → ci-ok    release(tag) → npm OIDC   │
                │ preview(label) → pkg-pr-new                 │
                └───────┬───────────────────────────┬─────────┘
          npm @vestra-player/sdk@x.y.z               │ 每周 / SDK 发布后 dispatch
                        ▼                            ▼
┌──────────── Repo A (private) ───────────┐   ┌──── Repo C (public plugins) ────┐
│ ci.yml                                  │   │ ci.yml                           │
│  plan ─▶ static[ubuntu] ─┬▶ windows     │   │  changes ─▶ _plugin.yml × N      │
│                          └▶ macos(条件) │   │          ─▶ index dry-run sign   │
│                          ─▶ ci-ok       │   │          ─▶ ci-ok                │
│ nightly: audit + full + unsigned pkg    │   │ publish(main): build ─▶ sign     │
│ release(tag): 同一套 _static/_build-test │   │   (env: plugin-index) ─▶ attest  │
│   ─▶ _package ─▶ sign(ubuntu rcodesign  │   │   ─▶ signed index                │
│      / Azure TS) ─▶ draft release       │   └──────────────────────────────────┘
└─────────────────────────────────────────┘
```

### 10.9 落地顺序建议
1. Repo A：先加 `_static.yml`（fmt、TS 脚本、SDK diff、cargo-deny licenses）、`_build-test.yml`（Windows）和 `ci-ok`，**先不接 macOS**（本地就是 macOS）。
2. 加 check-unsafe-policy、clippy `-D warnings`、typos、zizmor、actionlint。修完现有告警后再设为 required。
3. 加 macOS 的条件运行和 nightly（audit、full、未签名包）。
4. 加 cargo-about 生成第三方许可证文本，以及非 Cargo 依赖清单的检查。
5. release：先只做未签名的 draft release，再接 Windows Azure Trusted Signing，最后接 macOS rcodesign 签名和公证（需要 Developer ID 证书和 ASC API key）。
6. Repo B：迁到 npm trusted publishing，加 publint、attw、typedoc，并设置 environment。Repo C：加插件矩阵、签名 dry-run，并把真实私钥移进 environment。

---

### 附：引用文件清单（均为各仓库 HEAD）
- slint-ui/slint：`.github/workflows/{autofix,ci,build_and_test_reusable,build_docs,schedule_ci,notify_ci_failure,schedule_nightly_snapshot,nightly_snapshot,pr_checks,publish_npm_package,visual_editor_macos_dmg,visual_editor_windows_msix}.yaml`、`.github/actions/{setup-rust,codesign}/action.yaml`、`.github/ci_path_filters.yaml`、`scripts/run_clippy.sh`、`.clippy.toml`、`.mise/tasks.toml`
- langchain-ai/langgraph：`.github/workflows/{ci,_lint,_test,_test_release,release,pr_lint}.yml`
- langchain-ai/langgraphjs：`.github/workflows/{ci,release,test-exports}.yml`、`package.json`
- tauri-apps/tauri：`.github/workflows/{fmt,lint-rust,lint-js,audit,supply-chain,udeps,check-license-header,test-core,test-cli-rs,covector-version-or-publish,publish-cli-js,publish-cli-rs}.yml`；tauri-apps/tauri-action：`examples/publish-to-auto-release-universal-macos-app-with-signing-certificate.yml`
- denoland/deno：`.github/workflows/{ci.ts,ci.generated.yml,pr.generated.yml,npm_publish.generated.yml,promote_to_release.generated.yml,start_release.generated.yml}`、`tools/lint.js`
- BurntSushi/ripgrep：`.github/workflows/{ci,release}.yml`
- tokio-rs/tokio：`.github/workflows/{ci,audit,pr-audit}.yml`、`deny.toml`
- astral-sh/uv：`.github/workflows/{ci,plan,check-lint,check-zizmor,check-crates-policies,test,release,sign-release-binaries}.yml`、`Cargo.toml [workspace.lints]`、`.cargo/deny.toml`；astral-sh/github-actions：`sign-macos/action.yml`、`sign-windows/action.yml`
- vitejs/vite：`.github/workflows/{ci,publish,zizmor,semgrep,preview-release}.yml`、`package.json`
