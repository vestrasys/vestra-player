# GitHub Actions CI structure in open-source projects + CI plan for Vestra Player

Research date: 2026-10-22. All findings come from the current HEAD files fetched with `gh api repos/<o>/<r>/contents/.github/workflows/...` (raw copies cached in `scratchpad/wf/<owner>_<repo>/`). Paths are relative to the root of the repository in question.
Items marked [inference] are my own judgment and do not appear in the files studied.

---

## 0. One-page summary

| Pattern | Who uses it (files) | Value for us |
|---|---|---|
| **The entry workflow only orchestrates; check logic lives in reusable `workflow_call` workflows** | uv `ci.yml` → `check-*.yml`/`test.yml`; langgraph `ci.yml` → `_lint.yml`/`_test.yml`; slint `autofix.yaml` → `ci.yaml` → `build_and_test_reusable.yaml` | PR, main and release share one set of checks |
| **Run cheap static checks first, then release the expensive OS matrix** (`needs:` as a gate) | tokio `ci.yml`: `basics` needs fmt/clippy/docs/minrust, and the other ~40 jobs all `needs: basics`; slint `autofix.yaml`: `ci` needs `format_fix, lint_typecheck`; langgraphjs `ci.yml`: test needs lint/format/build | macOS is billed at 10x in private repos, so this is a must |
| **One plan/changes job computes outputs; later jobs switch on and off with `if:` by path/label** | uv `plan.yml` (paths + PR labels such as `test:macos`); slint `.github/ci_path_filters.yaml` + `dorny/paths-filter`; deno `pre_build` (skips docs-only changes and draft PRs) | The core cost-saving mechanism |
| **PRs skip expensive OSes by default; they run only on main, tags, or with a label** | deno `ci.ts`: on a PR without the `ci-full` label, the macOS/Windows release builds switch their runner to ubuntu and are skipped; uv `plan.yml`: `test:macos` label; slint `schedule_ci.yaml`: 3 times a day runs `full=true` on branches with new commits | Copy directly |
| **A final aggregate job for branch protection** (`if: always()`, fails if any need is failure/cancelled) | slint `autofix.yaml` `done`; langgraph `ci.yml` `ci_success`; uv `ci.yml` `required-checks-passed`; deno `ci-status`; vite `test-passed`/`test-failed` | Only one required check |
| **Re-verify before release, and split build and publish into two jobs** (only the publish job gets OIDC or write permissions) | langgraph `release.yml` (comment cites pypa: build and publish in separate jobs); uv `release.yml`; deno `npm_publish.ts` (publishes to a local Verdaccio for a smoke test before npm) | Keeps a poisoned dependency from reaching publish credentials |
| **Signing lives in an `environment`, effective only for main or tags** | deno: `environment: build` only on main/tag; uv `sign-release-binaries.yml`: `environment: release`; vite `publish.yml`: `environment: Release` | Secret isolation |
| **macOS signing and notarization done on Linux** (rcodesign + cloud HSM) | uv `sign-release-binaries.yml`: `sign-macos` runs on ubuntu using `astral-sh/github-actions/sign-macos` (rcodesign + Azure Key Vault + Apple notarization); deno uses `rcodesign sign` | Notarization usually waits 5–30 minutes; on a macOS runner that is billed at 10x |
| **Windows signing with Azure Trusted/Artifact Signing + OIDC** | deno `ci.ts` (`Azure/artifact-signing-action` followed by `signtool verify /pa`); uv `sign-windows` | No pfx certificate file needed |
| **npm with OIDC trusted publishing or provenance** | vite `publish.yml` (`id-token: write`, npm ≥ 11.5.2); langgraphjs `release.yml` (changesets + `NPM_CONFIG_PROVENANCE: true`); tauri `publish-cli-js.yml` (`NODE_AUTH_TOKEN: ''`, uses OIDC) | Use directly in Repo B |
| **Rust supply-chain checks** | tokio `audit.yml` (daily) + `pr-audit.yml` (only when a PR changes `Cargo.toml`), `EmbarkStudios/cargo-deny-action`; uv `check-lint.yml` (`cargo deny check licenses/sources/bans` + cargo-shear + typos); tauri `audit.yml` (rustsec/audit-check + pnpm audit, daily), `supply-chain.yml` (cargo-vet) | We must ship third-party licenses with the package, so cargo-deny's licenses check is a hard requirement |
| **Scan the workflows themselves for security problems** | uv `check-zizmor.yml`; vite `zizmor.yml`, plus a weekly semgrep run | Cheap; should be added |
| **Pin every action by SHA** | uv, langgraph, vite, deno (`# gagen:pin` converts tags to SHAs automatically) | Required on the release path |

---

## 1. slint-ui/slint (Rust + Slint, many target platforms)

### 1.1 Files and triggers
- `autofix.yaml`: **the actual PR entry point**. `on: pull_request (master, feature/*, pre-release/*) / workflow_dispatch / merge_group`, `concurrency: ci-${{ PR number || ref }}`, `cancel-in-progress: true`, top-level `permissions: contents: read`.
- `ci.yaml`: `on: workflow_call + workflow_dispatch`, inputs `full` (bool) and `changes` (JSON list of the paths-filter groups that matched). It **does not listen to push or PR** itself.
- `schedule_ci.yaml`: `cron '18 4,11,18 * * *'`. Iterates over `master|feature/*|pre-release/*` branches and runs `gh workflow run CI -f full=true` whenever the branch tip differs from the sha of the last CI run. It triggers with a GitHub App token, because a dispatch started by GITHUB_TOKEN does not trigger workflow_run.
- `notify_ci_failure.yaml`: `workflow_run` listens for failures of `CI` and `Nightly snapshot` on master and sends a notification.
- `schedule_nightly_snapshot.yaml`: `cron '18 2 * * *'`; dispatches `nightly_snapshot.yaml` only if there were commits in the last 24 hours (mode is `full-nightly`/`website-only`).
- `nightly_snapshot.yaml`: `workflow_dispatch(mode: release|full-nightly|website-only|test)`. Nightly and the official release use **the same file**.
- `pr_checks.yaml`: `pull_request_target`; the comment states *"never checks out or runs code from the pull request"*. It reads only the PR description and checks that the contributor agreement box is ticked.
- Reusable components: `build_and_test_reusable.yaml`, `node_test_reusable.yaml`, `python_test_reusable.yaml`, `build_docs.yaml`, `slint_tool_binary.yaml`, `wasm_*.yaml`, `cpp_package.yaml`, `upload_pypi*.yaml` (all `workflow_call`); composite actions are `.github/actions/setup-rust`, `install-linux-dependencies`, `codesign`, `install-sparkle`.

### 1.2 Job graph
```
autofix.yaml (PR / merge_group)
 ├─ format_fix      [ubuntu, 5min] paths-filter(.github/ci_path_filters.yaml) → outputs.changes
 │                   mise run ci:autofix:fix (cpp/py/rust/toml/ts formatting, license headers, pnpm dedupe) → autofix-ci commits suggestions
 ├─ lint_typecheck  [ubuntu, 5min] mise run ci:autofix:lint (REUSE, python, ts typecheck, knip) + cspell
 ├─ ci  needs[format_fix, lint_typecheck]  uses: ./ci.yaml  with{full:false, changes}
 │    every job in ci.yaml:  if: inputs.full || contains(fromJSON(inputs.changes), '<group>')
 │     ├─ build_and_test (matrix: windows-2022@1.95[MSRV], macos-14@beta, ubuntu…) → uses build_and_test_reusable.yaml
 │     ├─ clippy [ubuntu] ./scripts/run_clippy.sh  (all 4 workspaces with -D warnings)
 │     ├─ node_test_{linux,macos,windows} / python_test_* → uses *_reusable.yaml
 │     ├─ docs (only !full) → uses build_docs.yaml   (RUSTDOCFLAGS=-D warnings)
 │     ├─ cross_compile_checks / android / ios_cpp / esp-idf / wasm_demo …
 │     └─ [full only] full_build_and_test (cargo update + nightly), miri, win32_build(cargo xwin on ubuntu),
 │                    linuxkms_build_matrix, ios, yocto, zephyr, qa-tree-sitter-latest
 └─ done  needs[ci, format_fix, lint_typecheck]  if: always()
          → exit 1 if any result is failure or cancelled   (branch protection requires only this job)

schedule_ci.yaml (3x/day) ──dispatch──▶ ci.yaml(full=true) ──fail──▶ notify_ci_failure.yaml

schedule_nightly_snapshot.yaml ──dispatch──▶ nightly_snapshot.yaml(mode)
 ├─ cross_containers ─┬─ slint-viewer-binary / slint-lsp-binary (uses slint_tool_binary.yaml)
 │                    ├─ cpp_package / python_wheels
 ├─ docs (uses build_docs.yaml)   wasm_demo / wasm
 ├─ build_vscode_lsp_{linux_windows, macos_x86_64, macos_aarch64} → macos_bundle(lipo + ./.github/actions/codesign)
 ├─ publish_artifacts (mode != test)
 ├─ prepare_release (permissions: contents: write; ncipollo/release-action)
 ├─ publish_crates_io (id-token: write; rust-lang/crates-io-auth-action = crates.io trusted publishing; publish.sh --dry-run, then for real)
 └─ trigger_package_publishing → gh workflow run upload_pypi / publish_npm_package …
```

### 1.3 Static checks
- rustfmt: in `ci:autofix:fix` it **fixes directly and autofix.ci commits the fix**, rather than only reporting an error.
- clippy: `scripts/run_clippy.sh` runs `cargo clippy --locked --all-features --workspace … -- -D warnings` on the four workspaces root/examples/demos/tests; after a failure it keeps running the remaining workspaces and exits once at the end. Runs only on ubuntu.
- `.clippy.toml`: `type-complexity-threshold`, `too-many-arguments-threshold`, and `disallowed-methods` (bans `tokio::task::spawn_local`). Takes effect only in crates that write `#![deny(clippy::disallowed_methods)]`.
- MSRV: the build_and_test matrix pins Windows to `rust_version: "1.95"`, macOS uses beta, and full mode adds nightly + `cargo update`.
- miri: only in full mode (`cargo miri test -p vtable -p const-field-offset -p i-slint-common`, `-p i-slint-core --lib`).
- Licenses: `lint:legal:reuse` (REUSE spec) and `cargo xtask check_license_headers`.
- TS: biome, knip, typecheck. Spell checking with cspell.
- **No** cargo-deny, cargo-audit, zizmor or CodeQL. Actions are mostly pinned by tag (`@v7`); only `autofix-ci/action` is pinned by SHA.

### 1.4 Docs
`build_docs.yaml` (workflow_call): `cargo doc --no-deps --all-features`, `RUSTDOCFLAGS: -D warnings …`, and also builds doxygen and astro. PRs call it only when the `docs_build` group changes; nightly runs it every time.

### 1.5 Caching and cost
- `.github/actions/setup-rust` (composite): `dtolnay/rust-toolchain` + `Swatinem/rust-cache@v2`, **`save_if` defaults to `github.ref == 'refs/heads/master'`**: only master writes the cache, PRs only read it.
- On Windows: `RUSTFLAGS=-C debuginfo=0`, `git core.longpaths`, `CARGO_HOME` moved to a short path. Globally `CARGO_INCREMENTAL=false`, `CARGO_PROFILE_DEV_DEBUG=0`.
- Path groups in `.github/ci_path_filters.yaml` are composed with YAML anchors (`ci_config`, `compiler`, `internal`…). Changing `Cargo.lock` counts as a `ci_config`-level change and triggers every job.
- The full test run (cargo update, nightly, all targets) does not run on PRs; the 3-times-daily schedule handles it.

### 1.6 Release and signing
- `nightly_snapshot.yaml` handles both nightly and release, distinguished by `mode`. It reuses the **build-type** reusables (tool binary, docs, wasm) and does **not** rerun ci.yaml; quality is guaranteed by the earlier scheduled full CI.
- macOS: `visual_editor_macos_dmg.yaml` works in steps through `scripts/package_macos_visual_editor.bash`: `install-signing-material → archive-app → stage-and-sign-app → notarize-and-staple-app`; the DMG is notarized too. The `codesign` composite is the standard approach: temporary keychain + `security import p12` + `codesign --force -s`.
- Windows: `visual_editor_windows_msix.yaml` builds an MSIX and submits it to the Store with `microsoft/microsoft-store-apppublisher`; the Store signs it.
- crates.io: `rust-lang/crates-io-auth-action` (OIDC), `--dry-run` first.
- npm: `publish_npm_package.yaml` sets `permissions: id-token: write`.

### 1.7 Security
Top-level `permissions: contents: read`; individual jobs raise permissions (for example `deployments: write`). `pull_request_target` reads only metadata and does not check out PR code. schedule/dispatch check `github.repository == 'slint-ui/slint'` so they do not run in forks.

---

## 2. langchain-ai/langgraph (Python monorepo) + langchain-ai/langgraphjs (TS monorepo)

> Note: `libs/sdk-js` in the langgraph repository contains only a README; the JS code moved to `langgraphjs`, so both are covered together below.

### 2.1 langgraph `ci.yml`
- `on: workflow_dispatch / push: main / pull_request`, `permissions: contents: read`, `concurrency: ${{workflow}}-${{ref}}` cancel.
```
changes [dorny/paths-filter] → outputs python/deps/sdk_py  (on dispatch, `|| 'true'` runs everything by default)
 ├─ lint   matrix working-directory × 8  → uses ./_lint.yml  {working-directory}
 ├─ test   matrix working-directory × 7  → uses ./_test.yml  (inner matrix python 3.10–3.14)
 ├─ test-langgraph → uses ./_test_langgraph.yml  (different matrix, so split out)
 ├─ check-sdk-methods / check-schema (regenerates the schema and diffs; fails on mismatch)
 ├─ integration-test → uses ./_integration_test.yml
 ├─ sdk-py-integration-test → uses ./_sdk_integration_test.yml
 └─ ci_success  needs[all]  if: always()
      EXIT_CODE = !contains(needs.*.result,'failure') && !contains(...,'cancelled') ? 0 : 1
```
- `_lint.yml` (workflow_call, input `working-directory`): runs only one Python version (the comment says that versions other than min/max almost never add lint problems, and this saves concurrency). Inside the job it filters `${dir}/**` with `Ana06/get-changed-files`; **if the package did not change, all steps are skipped**, the job result is still success, and the aggregate job is unaffected. Commands go through the package's own `make lint_package` / `make lint_tests`.
- `pr_lint.yml`: `amannn/action-semantic-pull-request` checks the PR title, `permissions: pull-requests: read`.
- All third-party actions are pinned by SHA with the version in a comment (for example `actions/checkout@3d3c42e…# v7.0.1`).

### 2.2 langgraph `release.yml` (how reuse works)
`on: workflow_dispatch(working-directory)`; each package is published separately:
```
build (uv build, upload dist) ─▶ release-notes ─▶ pre-release-checks
   (installs the freshly built wheel and runs make test: it tests the artifact itself, not the source)
      ─▶ test-pypi-publish  uses ./_test_release.yml  (permissions id-token: write) → TestPyPI
      ─▶ publish  (permissions: id-token: write only; pypa/gh-action-pypi-publish = trusted publishing)
      ─▶ mark-release (permissions: contents: write; ncipollo/release-action)
```
Key point: the release **does not rerun** `_lint`/`_test`; it reruns tests against the build artifact. The build job has no write permissions at all; the comment reads *"a malicious build step (e.g. via a compromised dependency) could get access to our GitHub or PyPI credentials"*.

### 2.3 langgraphjs
- `ci.yml`: `lint` (oxlint), `format` (oxfmt --check), `build` (turbo; the artifacts are packed into a tarball and uploaded, then `pkg-pr-new publish` publishes a PR preview package) → `test` needs[lint, format, build]: ubuntu runs node 22/24/26, windows and macos **run only one node version**. `browser-tests` and `environment-tests` are reused via `uses: ./test-browser.yml`, `./test-exports.yml` with an `artifact-name` input, **reusing the build artifact instead of rebuilding**.
- `test-exports.yml`: imports the artifact in docker with esm/cjs/esbuild/vite/tsc/cloudflare/vercel to validate the exports map. This is a home-grown version of "are-the-types-wrong".
- `release.yml`: `on: push main`; `changesets/action` opens the version PR or publishes automatically, `NPM_CONFIG_PROVENANCE: true`, `id-token: write`.

---

## 3. tauri-apps/tauri (Rust + JS, desktop framework)

### 3.1 Files and triggers: many small workflows, each with its own path filter
| File | Trigger | Content |
|---|---|---|
| `fmt.yml` | PR | `cargo fmt --all -- --check`, prettier `pnpm format:check`, taplo `fmt --check` |
| `lint-rust.yml` | push dev; PR changing `crates/**`, `Cargo.*` | clippy matrix (windows/linux/macos/ios/android targets): `clippy --all-targets --all-features -- -D warnings`, with `Swatinem/rust-cache` |
| `lint-js.yml` | PR changing `packages/**` | eslint; `pnpm build && pnpm ts:check` |
| `test-core.yml` | push dev; PR (path include/exclude lists) | matrix: OS×target, toolchain **pinned to 1.90 (MSRV)**, cross for mobile |
| `test-cli-rs.yml` | same as above | `dtolnay/rust-toolchain@1.90` |
| `audit.yml` | `schedule 0 0 * * *` / dispatch; PR and push only when lock or manifest files change | `rustsec/audit-check@v2` (opens an issue when it finds a vulnerability, hence `issues: write`) + `pnpm audit` |
| `supply-chain.yml` | daily; push dev with lock changes | `cargo vet suggest` (`cargo vet --locked` is commented out, so it is not enforced yet) |
| `udeps.yml` | push dev | changes → setup (generates the crate matrix dynamically) → `cargo udeps` (nightly) |
| `check-license-header.yml` | PR | checks license headers only on **added files** (paths-filter `added_files`) |
| `covector-version-or-publish.yml` | push dev | release orchestration, see below |

Every workflow sets `concurrency: ${{workflow}}-${{ref}}` cancel and `permissions: contents: read`. Actions are pinned by tag (`@v7`), not SHA.

### 3.2 Release orchestration (rerunning checks before release)
```
push dev ─▶ check-publish (.changes/*.md exists → publish=true)
   ├─ audit (cargo audit + pnpm audit)
   ├─ run-integration-tests  matrix ubuntu/macos/windows
   ├─ test-api-e2e         uses ./test-api-e2e.yml          (reusable shared with PRs)
   ├─ test-api-e2e-mobile  uses ./test-api-e2e-mobile.yml
   └─ version-or-publish  needs[above] if: !cancelled() && !failure()
        covector → opens the version PR or publishes; then repository_dispatch / gh workflow run
            ├─▶ publish-cli-js.yml : build(matrix 10+ targets, napi) → test bindings(mac/win/linux×node22/24) → npm publish (id-token: write, NODE_AUTH_TOKEN='')
            └─▶ publish-cli-rs.yml : build matrix → upload (softprops/action-gh-release)
```

### 3.3 Desktop app signing (official tauri-apps/tauri-action example)
`examples/publish-to-auto-release-universal-macos-app-with-signing-certificate.yml`:
- Temporary keychain: `security create-keychain`, `set-keychain-settings -t 3600`, `import certificate.p12 -T /usr/bin/codesign`, `set-key-partition-list`, then `find-identity` to get the `Developer ID Application` identity.
- `tauri-action` environment variables: `APPLE_ID`, `APPLE_PASSWORD` (app-specific password), `APPLE_TEAM_ID`, `APPLE_CERTIFICATE`, `APPLE_SIGNING_IDENTITY` (it performs notarization), `releaseDraft: true`.
- Job-level `permissions: contents: write`.

---

## 4. denoland/deno (Rust + TS, binary distribution)

### 4.1 Workflows generated from TS
- `ci.ts` → `ci.generated.yml` (the tool is `jsr:@david/gagen`). The end of the file has `# gagen:pin actions/checkout@v6 = de0fac2e…`; **tags are replaced with SHAs automatically at generation time**. `ensureWorkflowYmlsUpToDate()` in `tools/lint.js` checks that the generated files are up to date.
- `ci.generated.yml`: `on: push main + tags '*' / pull_request(opened, reopened, synchronize, ready_for_review)`. `concurrency` groups by head_ref; with the `ci-test-flaky` label it groups by run_id instead (no cancellation).

### 4.2 Job graph and cost control
```
pre_build [ubuntu]  draft PR and commit message without [ci] → skip_build; tools/check_docs_only_changes.js → docs_only
 ├─ lint  matrix(linux/mac/windows)  format.js --check (linux only), jsdoc_checker, lint.js
 │        lint.js: dlint/JS lint, copyright headers, clippy(-D warnings + --deny clippy::print_stdout/print_stderr/
 │                 unused_async/large_futures/allow_attributes_without_reason),
 │                 ensureDisallowedMethodsEnforced(checks that every crate's clippy.toml contains the required list of disallowed methods),
 │                 ensureWorkflowYmlsUpToDate
 ├─ build-{debug,release}-{linux,macos,windows}-{x86_64,aarch64}  (environment: 'build' only on main/tag)
 │     PR without the ci-full label: mac/windows release builds switch runner to ubuntu-24.04 and skip
 │     → test-*-*  shards(shard_index/shard_total)
 ├─ deno-core-test / deno-core-miri / bench(only with the ci-bench label or on main)
 ├─ ci-status  needs[all] if: always()  → "Ensure CI success"
 └─ publish-canary  needs[all builds] if: denoland/deno && main
```
Also: the cache key includes a `cacheVersion` constant, changed to clear the cache; slow release-only steps such as the startup-order second link run only when `isMainOrTag || ci-full`.

### 4.3 Signing
- macOS: in the macOS build job, `rcodesign sign --code-signature-flags=runtime --p12-file=<(… base64 -d) --entitlements-xml-file=cli/entitlements.plist`, only on main or tags.
- Windows: first `azure/login@v2` (OIDC; the comment warns *Azure OIDC credentials are only valid for 5 minutes, so authentication must be done right before signing*), then `Azure/artifact-signing-action` (Trusted Signing; parameters include endpoint, account, certificate-profile, RFC3161 timestamp), and finally `signtool verify /pa /v` to verify.
- Release flow: `start_release` → `version_bump` → `create_prerelease_tag` → `promote_to_release` (`environment: build`, `id-token: write`; re-tags the canary artifacts with the version and re-signs them).
- npm: `npm_publish.generated.yml`: build → test (**publish to a local Verdaccio first, then `npm install -g` for a smoke test**) → publish (`permissions: id-token: write`, with a `dry_run` input).
- Security: the comment at the top of `pr.generated.yml` warns that GITHUB_TOKEN has write permissions in a PR context. The PR title check just `deno run`s a remote script and does not check out PR code.

---

## 5. BurntSushi/ripgrep (Rust CLI, the classic release-build setup)

- `ci.yml`: `on: pull_request / push master / schedule '00 01 * * *'`, `permissions: contents: read`.
  - `test` matrix: `pinned 1.96.0` (MSRV), stable, beta, nightly, musl, x86, several cross targets (aarch64/powerpc/s390x…), macOS, Windows msvc/gnu.
  - `rustfmt` (`cargo fmt --all --check`), `docs` (`cargo doc --no-deps --document-private-items --workspace`), `wasm`, `fuzz_testing` (only `cargo check`).
  - No caching, no aggregate job, no clippy.
- `release.yml`: `on: push tags "[0-9]+.[0-9]+.[0-9]+"`; `permissions: contents: write, id-token: write, attestations: write`.
```
create-release : verify tag == Cargo.toml version → gh release create $V --draft --verify-tag
   ├─ build-release (matrix 10+ targets, cross v0.2.5 pinned version, --profile release-lto, strip,
   │                 package zip/tar.gz + sha256 + actions/attest-build-provenance(SHA pinned) → gh release upload)
   └─ build-release-deb (cargo deb)
the draft is checked by a person before publishing
```

---

## 6. tokio-rs/tokio (Rust library, the most complete static checks)

- `ci.yml`: `on: push (master, tokio-*.x) / pull_request`, `permissions: contents: read`, concurrency cancel. Global `env: RUSTFLAGS: -Dwarnings`, with every toolchain pinned as a variable: `rust_clippy: '1.88'`, `rust_min: '1.85'`, `rust_nightly: nightly-2025-10-12`, `rust_miri_nightly`…
```
fmt ─┐
clippy (pinned 1.88, --workspace --tests --no-deps, two passes: stable features / all-features+unstable) ─┤
docs (windows+ubuntu, RUSTDOCFLAGS: --cfg docsrs -Dwarnings) ─┤
minrust (1.85 cargo check) ─┘
   └─▶ basics (run: exit 0)   ← gate for the cheap checks
         └─▶ ~40 jobs: test-tokio-full ×3 OS, all-features, panic-abort, miri-lib/test/doc, asan, valgrind,
                       semver (obi1kenobi/cargo-semver-checks-action), minimal-versions (cargo update -Z minimal-versions),
                       cross-check, wasm32-*, check-external-types, check-spelling (cargo-spellcheck + dictionary sort check),
                       loom-compile, freebsd, io-uring kernel matrix (uses ./uring-kernel-version-test.yml)
```
- `audit.yml`: `push master paths **/Cargo.toml` + `schedule 0 2 * * *` → `EmbarkStudios/cargo-deny-action@v2` (job permissions `checks: write, issues: write`).
- `pr-audit.yml`: runs cargo-deny only when a PR or push changes `**/Cargo.toml`. **Advisories do not run on every PR**, so a newly published vulnerability does not turn unrelated PRs red; the daily scheduled job finds new vulnerabilities.
- `deny.toml`: `[licenses] allow = ["MIT","Apache-2.0"]` + `exceptions = [{allow=["Unicode-3.0"], crate="unicode-ident"}]`; `[bans] wildcards="deny"`; `[sources] unknown-registry/unknown-git = "deny"`.

---

## 7. astral-sh/uv (Rust, the best orchestration and release security)

### 7.1 `ci.yml` only orchestrates
`on: push main / pull_request / workflow_dispatch`, `permissions: contents: read`, concurrency cancel.
```
plan (uses ./plan.yml)  → from changed files + PR labels computes: test-code, test-macos, run-checks, save-rust-cache,
                          build-release-binaries, check-crates-policies …
                          labels: test:skip test:macos test:integration test:extended build:release build:skip …
 ├─ check-fmt / check-lint / check-docs / check-zizmor / check-lock / check-publish / check-release
 │  check-generated-files / check-crates-policies   (all uses ./check-*.yml)
 ├─ test (uses ./test.yml, with test-macos)   ← macOS tests are off by default; on via plan output or the test:macos label
 ├─ build-dev-binaries ─▶ test-smoke / test-integration / test-system / test-ecosystem (reuse the dev binaries)
 ├─ build-release-artifacts (uses ./build-release-binaries.yml, only with the build:release label)  ← the same file as release.yml uses
 └─ required-checks-passed  if: always() && ref != main
      jq iterates over needs and fails if a result is not success/skipped; a CONDITIONAL_JOBS table declares which jobs may be skipped conditionally
```
- `check-lint.yml`: ruff, ty, shellcheck, validate-pyproject, `clippy-ubuntu` and `clippy-windows` (`cargo clippy --workspace --all-targets --all-features --locked`), **cargo-deny** (`check licenses`, `check sources`, `scripts/check-cargo-bans.py`, `check bans`), **cargo-shear --deny-warnings** (unused dependencies), **typos**, hawk (`-D warnings`).
- Lint configuration lives in `Cargo.toml [workspace.lints]`: `rust.unsafe_code = "warn"`, `unreachable_pub = "warn"`; `clippy.pedantic = {level="warn", priority=-2}` plus a set of allows, and `print_stdout`/`print_stderr`/`dbg_macro`/`exit`/`get_unwrap`… as warn. The CI environment turns warnings into errors.
- `.cargo/deny.toml`: `[bans] wildcards="deny"`, **`[bans.build] allow-build-scripts = [...]` (allowlist of dependencies permitted to have build.rs)**.
- `check-zizmor.yml`: `zizmorcore/zizmor-action`, `security-events: write`, results uploaded to code scanning.
- `check-crates-policies.yml`: top-level `permissions: {}`, job-level `environment: automations`, `id-token: write`; reviews newly added crates against an external policy repository.
- Caching: input `save-rust-cache`; plan decides whether to write the cache (mainly main).

### 7.2 `release.yml` (reuses the CI build, signing kept separate)
`on: workflow_dispatch(dry-run)`, **top-level `permissions: {}`**, `ACTIONS_CACHE_MODE: none` (release uses no cache, to prevent cache poisoning).
```
release-gate (environment: release-gate → manual approval)
plan (cargo-dist plan)
 ├─ build-release-artifacts  uses ./build-release-binaries.yml   (the same reusable as CI's build:release)
 ├─ build-docker            uses ./build-docker.yml
 └─ assemble-release-artifacts  needs[plan, build, release-gate]  uses ./sign-release-binaries.yml
        prepare ─┬─ sign-macos   [ubuntu runner!] environment: release, id-token: write
                 │     astral-sh/github-actions/sign-macos: rcodesign + Azure Key Vault PKCS#11 + Apple notarization
                 ├─ sign-windows [windows-2025]   environment: release, azure/artifact-signing-action
                 └─ assemble → verify-macos / verify-windows (verify the signed artifacts on the real OS)
 → generate-local-dist-manifest → build-global-dist-artifacts → finalize-dist-manifest
 → publish-pypi / publish-crates / publish-docker (attestations: write)
 → publish-github (actions/attest-build-provenance) → publish-docs / publish-versions / publish-mirror
```
Every job declares its own minimal permissions, and every action is pinned by SHA.

---

## 8. vitejs/vite (TS)

- `ci.yml`: `on: push (main, release/*, feat/*, fix/*, perf/*, v*) / pull_request / dispatch`, **top-level `permissions: {}`**, concurrency cancel.
```
changed [ubuntu-slim] tj-actions/changed-files → should_skip(docs-only changes)
 ├─ test needs changed  matrix: ubuntu × node 20/22/24/26 + macos@24 + windows@24 (only 1 each)
 │     build → test-unit → test-serve → test-serve-bundled → test-build
 ├─ test-passed (if !cancelled && !failure) / test-failed   ← for branch protection
 └─ lint [ubuntu]: pnpm build, oxlint, oxfmt + git diff --exit-code, tsc typecheck (tsc -p scripts && pnpm -r typecheck), test-docs
```
- `zizmor.yml`: PR / push main (`.github/workflows/**`), `security-events: write`.
- `semgrep.yml`: runs weekly; results uploaded to code scanning as SARIF.
- `publish.yml`: `on: push main / v*`, `detect-release` (decides from the commit message whether to publish) → `publish-release`: `environment: Release`, `permissions: contents: write, id-token: write`, **`npm i -g npm@^11.5.2` (npm trusted publishing requires ≥ 11.5)**, `pnpm run ci-publish` (`@vitejs/release-scripts`). Also `yq '.allowBuilds[]=false'`: dependencies may not run build scripts during publishing.
- `preview-release.yml`: PRs with the `trigger: preview` label publish a preview package with `pkg-pr-new`.

---

## 9. Side-by-side comparison

| Item | slint | langgraph(js) | tauri | deno | ripgrep | tokio | uv | vite |
|---|---|---|---|---|---|---|---|---|
| Entry + reusable | ✔ autofix→ci→*_reusable | ✔ _lint/_test | partial | generator | ✗ | a few | ✔✔ | ✗ |
| Aggregate job | done | ci_success | ✗ | ci-status | ✗ | basics(upfront gate) | required-checks-passed | test-passed |
| Path filtering | paths-filter groups | paths-filter + per-package changed-files | on.paths | docs-only script | ✗ | on.paths(audit) | plan.yml | changed-files |
| Expensive OS policy | full only on schedule | mac/win run only one node version | runs all | skipped on PRs without label | runs all | runs all | test:macos label | mac/win run only one version |
| clippy -D warnings | ✔(ubuntu) | – | ✔(5 targets) | ✔ + extra deny | ✗ | ✔(RUSTFLAGS) | ✔(ubuntu+win) | – |
| cargo-deny/audit | ✗ | – | audit + vet | – | ✗ | deny(daily + on change) | deny licenses/sources/bans | – |
| MSRV | 1.95 | – | 1.90 | rust-toolchain | 1.96.0 | 1.85 | rust-toolchain.toml | – |
| Unused dependencies | knip(TS) | – | udeps | – | – | – | cargo-shear | – |
| Spelling | cspell | – | – | – | – | cargo-spellcheck | typos | – |
| Workflow audit | ✗ | – | ✗ | generated-file check | ✗ | ✗ | zizmor | zizmor + semgrep |
| SHA pinning | rarely | ✔ | ✗ | ✔(automatic) | partial | ✗ | ✔ | ✔ |
| Signing | codesign + notarize(mac script), MSIX Store | – | tauri-action | rcodesign + Azure TS | – | – | rcodesign + notarization on Linux; Azure | – |
| Provenance | crates OIDC | npm provenance | npm OIDC | npm OIDC | attest-build-provenance | – | attest + OIDC | npm OIDC |
| Draft release | ✔ | – | releaseDraft | ✔ | ✔ --draft --verify-tag | – | dist | – |

---

## 10. Best-practice patterns and the recommended plan for Vestra Player

### 10.0 Constraints (from the current state of the repositories)
- Repo A (`vestra-player`, private): `src/host` is the Rust package `vestra-host` (GPL-3.0-or-later, edition 2024), `[workspace] members = ["unsafe/pip","unsafe/render","unsafe/win32"]`; host already has `[lints.rust] unsafe_code = "forbid"`. Dependencies include libmpv (`rsmpv`), Slint 1.18, objc2 (macOS), windows-registry and others. `src/package.json` already has `check`(tsc), `format:check`(prettier), `boundaries`, `portability`, `test:sdk`, `plugins:build`, `sdk:sync-slint`. 18 plugins in total, under `src/plugins/*`.
- There is no `.github/workflows` yet.
- Billing: private repos are Linux 1x, Windows 2x, macOS 10x. The main development machine is macOS.
- Platforms: macOS + Windows now, Linux/Android/iOS later. The host **cannot compile on Linux** (a lot of `cfg(target_os)` platform code), so clippy and tests must run on the target OS, or be cross-checked.

### 10.1 General principles (distilled from the projects above)
1. **The entry workflow only orchestrates** (uv/slint/langgraph): `ci.yml` and `release.yml` contain only `uses: ./.github/workflows/_xxx.yml`; the check logic is written once.
2. **Static checks on ubuntu act as the gate** (tokio `basics`, slint `needs[format_fix, lint_typecheck]`): if checks such as fmt, cargo-deny, TS and the unsafe policy fail, the Windows and macOS builds do not start.
3. **A plan job decides what runs** (uv `plan.yml`): outputs `run-macos`, `run-windows` etc. from paths, branch and labels.
4. **macOS does not run by default; it is switched on by condition** (deno `ci-full`, uv `test:macos`, slint scheduled full).
5. **Only one required check**: `ci-ok`, `if: always()`, fails if any need is failure or cancelled. Skipped jobs count as passed (slint `done`, uv `required-checks-passed`).
6. **Only main writes the cache** (slint `save_if`, uv `save-rust-cache`). Release uses no cache (uv `ACTIONS_CACHE_MODE: none`).
7. **Separate build from publish/sign** (langgraph, uv). Signing secrets live in `environment: release`, restricted to tag deployments, with manual approval (uv `release-gate`).
8. **Top-level `permissions: {}` or `contents: read`, raised at job level**; every action pinned by SHA (uv/vite/langgraph) and maintained by Renovate/Dependabot; scan workflows with zizmor (uv/vite).
9. **Never check out PR code with `pull_request_target`** (slint `pr_checks.yaml` comment).
10. **Generated files must match the source**: regenerate first, then `git diff --exit-code` (langgraph check-schema, deno ensureWorkflowYmlsUpToDate, vite `format && git diff --exit-code`); this corresponds to our SDK copy check.

### 10.2 Repo A (private: Rust host + Slint + 18 TS plugins)

#### File layout
```
.github/
  actions/
    setup-rust/action.yml      # composite: rust-toolchain.toml + Swatinem/rust-cache(save-if: main) + Windows short paths/longpaths
    setup-node/action.yml      # composite: setup-node + npm ci (cache)
  workflows/
    ci.yml                     # entry: push(any branch) / pull_request / merge_group / workflow_dispatch(full)
    nightly.yml                # schedule: runs only if main has new commits → uses _audit + _build-test(full) + _package(sign=false)
    release.yml                # tag v* → reuses _static + _build-test → _package(sign) → draft release
    _plan.yml                  # workflow_call: paths/labels/ref → outputs
    _static.yml                # workflow_call: all ubuntu static checks
    _build-test.yml            # workflow_call(os, toolchain, full): clippy + test + doc, on the target OS
    _package.yml               # workflow_call(os, sign): release build + bundle + signing + artifacts
    _audit.yml                 # workflow_call: cargo-deny advisories (can also run on its own schedule)
  CODEOWNERS                   # src/host/unsafe/** needs extra review
deny.toml / clippy.toml / _typos.toml / rust-toolchain.toml / about.toml(cargo-about)
src/scripts/check-unsafe-policy.mjs (or xtask)
```

#### ci.yml job graph
```
ci.yml  on: push{branches:['**'], tags-ignore:['**']} / pull_request / merge_group / workflow_dispatch{full}
        permissions: {}   concurrency: ci-${{ github.event.pull_request.number || github.ref }} cancel-in-progress
        (a single-person private repo can use push only; add pull_request when status is needed on the PR page, and dedupe with concurrency)

plan  [ubuntu, 1min]  uses _plan.yml
  outputs: rust(host/**, Cargo.*), ts(plugins/runtime/scripts/package*), mac_only(platform/macos*, unsafe/pip, unsafe/render),
           win_only(platform/windows*, unsafe/win32), run_macos, run_windows, full
  run_macos = ref==main || event==merge_group || label 'ci:macos' || full || (mac_only && is a PR)
  run_windows = rust changed (any branch)          ← the local dev machine does not cover Windows, so it runs by default
 │
 ├─ static  [ubuntu, ~5–8min]  uses _static.yml          ← gate, billed at 1x
 │     Rust: cargo fmt --check · taplo fmt --check · typos
 │           cargo deny check licenses bans sources (multiple targets [graph].targets, see below)
 │           cargo-shear (unused dependencies, no compile needed)
 │           check-unsafe-policy (see 10.4)
 │           cargo xwin clippy --target x86_64-pc-windows-msvc -- -D warnings   ← cross-checks Windows on Linux (slint win32_build used cargo xwin) [optional, feasibility to be verified]
 │     TS:   npm ci · npm run check(tsc) · format:check · boundaries · portability · test:sdk · plugins:build
 │           sdk copy check: npm run sdk:sync-slint && git diff --exit-code
 │           SDK copy == @vestra-player/sdk@<lock version> on npm (diff after npm pack)
 │     CI:   actionlint · zizmor (only when .github/** changes, uv/vite)
 │
 ├─ windows [windows-2022, ~15min, 2x]  needs[plan, static] if run_windows   uses _build-test.yml{os}
 │     cargo clippy --locked --all-targets --all-features -- -D warnings
 │     cargo test --locked  (including the unsafe/* workspace members)
 │     cargo doc --no-deps --document-private-items  (RUSTDOCFLAGS=-D warnings)  ← Windows only, saves macOS time
 │
 ├─ macos   [macos-15 arm64, ~15min, 10x]  needs[plan, static] if run_macos   uses _build-test.yml{os}
 │     clippy -D warnings · cargo test   (doc not repeated)
 │
 └─ ci-ok   needs[plan, static, windows, macos]  if: always()
       contains(needs.*.result,'failure'||'cancelled') → exit 1     ← the only required check for branch protection
```
Additional notes:
- **Make sure macOS has run before merge**: on PR merge or merge_group, `run_macos=true`; ordinary pushes to feature branches rely on `npm run ci:local` run on the local macOS dev machine. Add a script to `package.json` that runs the same commands as `_static` + `_build-test`, so local and CI stay consistent.
- Global env (slint/tauri): `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_TERM_COLOR=always`; every job sets `timeout-minutes`.
- Caching: `Swatinem/rust-cache` with `save-if: ${{ github.ref == 'refs/heads/main' }}`, key separated by OS. The repository cache limit is 10GB, and one target directory each for macOS and Windows easily exceeds it, so only main writes.

#### nightly.yml (schedule, 01:xx UTC; following slint's `schedule_ci`: full run only when the main tip has changed)
```
_audit         cargo deny check advisories  (runs every day, because RustSec adds new vulnerabilities; PRs run it only when Cargo.lock changes, see tokio pr-audit)
_build-test    macos + windows, toolchain=latest stable (an "upcoming version warning" beyond the version pinned in rust-toolchain.toml, like slint beta / ripgrep beta)
_build-test    test after cargo update (like slint full_build_and_test); failures only notify, they do not block
_package       sign=false, produces unsigned nightly packages for GUI testing
→ on failure, notify via workflow_run or an issue (slint notify_ci_failure.yaml)
```

#### release.yml (reuses the same checks, signing kept separate)
```
on: push tags 'v*'  (+ workflow_dispatch{dry_run})   permissions: {}   no cache
verify   [ubuntu] tag == src/host/Cargo.toml version (ripgrep create-release)
static   uses _static.yml                       ┐ the same files as CI
windows  uses _build-test.yml{windows}          │
macos    uses _build-test.yml{macos}            ┘
package-mac [macos-15]   needs[...]  uses _package.yml{os: macos, sign: false}
     cargo build --release --locked → .app bundle (with libmpv dylib, THIRD_PARTY_LICENSES, plugins) → upload unsigned
package-win [windows]    uses _package.yml{os: windows}
     cargo build --release → installer(MSI/MSIX/NSIS) → signing:
       environment: release · id-token: write · azure/login(OIDC) → Azure/artifact-signing-action → signtool verify /pa
       (deno ci.ts / uv sign-windows)
sign-mac [ubuntu!]  environment: release · needs package-mac          ← the key cost saving (uv sign-macos)
     rcodesign sign --code-signature-flags=runtime --entitlements … (every dylib/executable inside the .app)
     rcodesign notary-submit --staple (App Store Connect API key)  → the notarization wait happens on a 1x runner
     → DMG signing + notarization
     alternative: codesign + xcrun notarytool directly in the macOS job (slint/tauri-action); simpler, but the wait is billed at 10x
verify-mac [macos-15, short] codesign --verify --deep --strict · spctl -a -vv (the uv verify-macos idea)
checksums  sha256sum → SHA256SUMS
attest     actions/attest-build-provenance   ⚠ artifact attestations in private repos require GitHub Enterprise Cloud [inference, based on GitHub docs];
           when unavailable, use signing + SHA256SUMS
release    gh release create $TAG --draft --verify-tag  (ripgrep)  → publish after a manual check
           (or add a separate publish job with manual approval via environment: release-publish, like uv release-gate)
```
Environment configuration: `release` allows deployments only from `v*` tags, with required reviewers enabled (in a single-person project this can serve as a confirmation step); it stores the Apple certificate or ASC API key and the Azure client/tenant/subscription id. Everyday CI cannot reach these secrets.

#### Rust check configuration
- **rustfmt** + **taplo** (tauri `fmt.yml`).
- **clippy**: `cargo clippy --locked --all-targets --all-features -- -D warnings` (tauri/slint). Put lints in `[workspace.lints]` in `Cargo.toml`; **do not turn on all of pedantic at once**. Follow uv: set `pedantic = {level="warn", priority=-1}` and then allow item by item, or pick only these: `dbg_macro`, `print_stdout`, `print_stderr` (once the host has a logging system), `allow_attributes_without_reason` (deno), `undocumented_unsafe_blocks` (unsafe crates), `unwrap_used` (optional).
- **clippy.toml**: `disallowed-methods` (slint/deno) can enforce the cross-platform rule at compile time, for example banning `std::process::Command` outside platform modules; `check-portability.mjs` already covers part of this.
- **MSRV**: Vestra is an application, not a library, so it **needs no MSRV matrix**. Pin the version with `rust-toolchain.toml` (uv, deno), and use the latest stable in nightly as an early warning.
- **cargo-deny** (required; both the uv and tokio configurations are good references):
```toml
[graph]
targets = ["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc"]   # add linux/android/ios later
all-features = true
[licenses]
# host is GPL-3.0-or-later: only allow licenses compatible with GPLv3
allow = ["MIT","Apache-2.0","Apache-2.0 WITH LLVM-exception","BSD-2-Clause","BSD-3-Clause","ISC","Zlib",
         "Unicode-3.0","MPL-2.0","BSL-1.0","CC0-1.0","0BSD","LGPL-2.1-or-later","LGPL-3.0-or-later",
         "GPL-3.0-or-later","OFL-1.1","Unlicense"]
confidence-threshold = 0.9
# declare crates with unclear licenses explicitly with [[licenses.clarify]]
# anything not in allow (GPL-2.0-only, AGPL, SSPL, commercial licenses) fails directly
[bans]
wildcards = "deny"
multiple-versions = "warn"
# optional: [bans.build] allow-build-scripts = [...]  (uv): new dependencies with build.rs must be reviewed
[sources]
unknown-registry = "deny"
unknown-git = "deny"
[advisories]
yanked = "deny"
```
  - Shipping licenses with the package: at release packaging time, use **cargo-about** (keep the accepted list in `about.toml` consistent with deny.toml) to generate `THIRD_PARTY_LICENSES.html/txt` and put it in the `.app` and the installer [inference: none of the projects studied generate a third-party license file in CI; slint uses REUSE + license header checks]. CI runs `cargo about generate` in `_static` **only to check that generation succeeds**, so a new dependency without license text is caught; the real generation happens in `_package`.
  - Non-Cargo dependencies (libmpv and its ffmpeg and other dynamic libraries, fonts, npm runtime dependencies): keep a `third_party/manifest.toml` listing name, version, license and text path. `check-third-party.mjs` checks that every entry has its text and that the list matches the dylib/dll files actually in the bundle.
- **Pick one of cargo-audit and cargo-deny**: both read the RustSec database, so `cargo deny check advisories` is enough. PRs run it only when `Cargo.lock` changes (tokio `pr-audit.yml`); a daily schedule runs it in full (tokio `audit.yml`, tauri `audit.yml`).
- **Unused dependencies**: cargo-shear (uv; no compile needed, fast). cargo-udeps needs nightly and has to compile (tauri); not recommended.
- **typos** (uv). **actionlint + zizmor** (uv/vite).
- **miri**: the unsafe crates are almost entirely FFI (AVKit, OpenGL/IOSurface, Win32), and miri cannot run FFI, so **do not add it**. Instead, add unit tests for the pure-Rust parts of the unsafe crates.

### 10.3 Cost estimate and strategy (private repository)
Assuming a cold-cache Windows build+test of 15 minutes, macOS 15 minutes, and ubuntu static checks of 6 minutes [inference, needs measurement]:

| Scenario | What runs | Billed minutes |
|---|---|---|
| Feature branch push (TS only) | plan + static | ≈ 7 |
| Feature branch push (Rust changed) | + windows | ≈ 7 + 30 = 37 |
| PR merge, main, merge_group | + macos | ≈ 37 + 150 = **187** |
| nightly (when there are new commits) | audit + mac + win + unsigned package | ≈ 250–350 |
| release | static + tests on both OSes + packaging + macOS verify only (signing and notarization on Linux) | ≈ 300–400; if notarization is waited on macOS, add 10×(5–30) |

Cost-saving measures, ordered by benefit:
1. macOS runs only on main, merge, the `ci:macos` label and nightly (deno/uv); local macOS covers the rest with `ci:local`.
2. The ubuntu gate first; on failure, Windows and macOS do not start (tokio basics).
3. Signing and notarization on ubuntu (uv `sign-macos` + rcodesign); the macOS runner only compiles and does a short verification.
4. Only main writes the cache; release uses no cache. Set `CARGO_INCREMENTAL=0` and `debug=0` (slint/tauri).
5. concurrency cancels old runs; every job sets `timeout-minutes`.
6. Path filtering: when only `doc/**` changes, plan runs nothing (deno docs_only, vite should_skip).
7. When Linux is added later: Linux is the cheapest platform for a full build, so clippy and most tests can move to Linux as the first compile gate (slint runs clippy only on ubuntu), with macOS and Windows running only platform-specific tests. Android/iOS get only `cargo check --target` cross-checks (slint `cross_compile_checks`), with full builds in nightly.

### 10.4 Allowing unsafe only in `src/host/unsafe/*`: how to enforce it
Current state: the host sets `[lints.rust] unsafe_code = "forbid"`, which applies to all targets of that package (lib/bin/tests/build.rs); the three unsafe crates are separate packages. The following layers still need to be added:

1. **Switch to workspace-level lints**: set `unsafe_code = "forbid"` in `[workspace.lints.rust]` and put the shared clippy configuration in `[workspace.lints.clippy]`; the host writes `[lints] workspace = true`. Cargo does not allow overriding individual lints alongside `workspace = true`, so **the three unsafe/* crates do not inherit**; each writes its full `[lints]`:
   ```toml
   [lints.rust]
   unsafe_op_in_unsafe_fn = "deny"
   missing_docs = "warn"
   [lints.clippy]
   undocumented_unsafe_blocks = "deny"      # every unsafe block needs a // SAFETY:
   multiple_unsafe_ops_per_block = "deny"
   missing_safety_doc = "deny"
   # + copy the workspace clippy set
   ```
2. **check-unsafe-policy script** (following `ensureDisallowedMethodsEnforced` in deno's `tools/lint.js`: iterate over every crate and check the required configuration), placed in `_static` and run on ubuntu:
   - List all workspace packages with `cargo metadata --no-deps`. For packages whose path is not under `src/host/unsafe/`, `lints.workspace` must be true or `unsafe_code = "forbid"` must be written explicitly; otherwise fail. This also catches a newly added crate that forgot the setting.
   - Match `\bunsafe\b` in every `.rs` file (with comments and strings removed); fail if a match is not under `src/host/unsafe/**`. This complements the compile-time forbid and also covers code that does not compile on the current platform, such as `#[cfg(windows)]`. **This point is key**: a forbid compile check run on macOS cannot see code in Windows branches.
   - Also check that `#[allow(unsafe_code)]`, `#![allow(unsafe_code)]` and `[lints] unsafe_code = "allow"` do not appear where they should not.
   - Every `unsafe {` block in unsafe/* must be preceded by `// SAFETY:` (clippy checks only the platform being compiled; the script can cover all platforms).
   - Allowlist: the list of unsafe crates is written in the script. Adding an unsafe crate requires changing both the script and CODEOWNERS, which is directly visible in the diff.
3. **CODEOWNERS**: `src/host/unsafe/** @zysoong`, plus the branch protection setting "Require review from Code Owners", to prepare for future collaborators.
4. **unsafe at the dependency level**: left to cargo-deny (sources/bans). For more strictness, `[bans.build] allow-build-scripts` (uv) can review dependencies with build.rs; cargo-vet (tauri `supply-chain.yml`) costs too much for a single-person project and is not recommended.

### 10.5 The TS part (plugins and runtime in Repo A)
All existing scripts go into `_static`: `tsc --noEmit`, `prettier --check`, `boundaries`, `portability`, `test:sdk`, `plugins:build`, `sdk:sync-slint && git diff --exit-code`.
Possible additions:
- **oxlint** (used by both vite and langgraphjs; fast, almost no configuration) or the typescript-eslint strict configuration for eslint.
- The 18 plugins can use langgraph's per-directory matrix pattern: `changes` computes the changed plugins → `matrix: plugin` → reuse `_plugin.yml` (tsc, unit tests, `.vxp` packaging, manifest schema validation). Plugin builds are fast, so **running all of them at once is also fine**; split them when it gets slow.
- SDK copy check (Repo A ↔ Repo B): Repo A locks the `@vestra-player/sdk` version, and CI runs `npm pack @vestra-player/sdk@$(node -p "require('./package-lock.json')…")` and diffs against the copy in the repository, to make sure the copy is identical to what is published on npm (same idea as langgraph check-schema).

### 10.6 Repo B (public TS SDK, published to npm)
```
ci.yml  on: push main / pull_request / merge_group     permissions: {}   concurrency cancel
 ├─ lint     [ubuntu] oxlint|eslint · prettier --check · tsc --noEmit · typos
 ├─ build    [ubuntu] tsc/tsup build → upload dist (langgraphjs: tests reuse the build artifact)
 │           publint · @arethetypeswrong/cli (attw --pack)  [inference: neither appears in the projects studied; langgraphjs achieves the same with multi-environment import tests in docker]
 │           npm pack --dry-run → check the files allowlist (nothing beyond src/test/map)
 │           typedoc --treatWarningsAsErrors (docs build; deployed to Pages on main)
 ├─ test     needs[lint, build]  matrix: ubuntu × node LTS(20/22/24) + windows@22 + macos@22  (vite/langgraphjs: non-Linux runs only one version)
 ├─ exports  needs build: real import tests for esm/cjs/types (langgraphjs test-exports.yml)
 ├─ zizmor   (only .github/** changed)
 └─ ci-ok    aggregator
preview.yml  'preview' label → pkg-pr-new publish (vite/langgraphjs); Repo A and Repo C can install this package first to try it

release.yml  on: push tags 'v*'   (or changesets/action opening the version PR automatically, like langgraphjs)
 ├─ verify: tag == package.json version (ripgrep)
 ├─ ci:   uses ./ci.yml  (ci.yml also declares workflow_call; a full rerun before publishing)
 ├─ build: npm pack → upload tgz          (no write permissions; langgraph's build/publish separation)
 ├─ smoke: install the tgz and run tsc on the example plugins (deno smoke-tests with a local Verdaccio registry)
 └─ publish  environment: npm (required reviewers, only v* tags)
       permissions: id-token: write, contents: write
       npm i -g npm@^11.5   (vite)  →  npm publish <tgz> (Trusted Publishing: bind repo + workflow + environment in the npmjs.com settings,
                                       no stored NPM_TOKEN, provenance attached automatically)
       gh release create (notes taken from CHANGELOG)
```
Other: every action pinned by SHA + Renovate/Dependabot; CODEOWNERS for `.github/workflows/**`; tag protection rules enabled; the npm package set to "require 2FA and disallow tokens", allowing publishing only through trusted publishing.

### 10.7 Repo C (public plugin repository, already publishes a signed index)
To add:
1. **PR CI as a per-plugin matrix** (langgraph `ci.yml` + `_lint.yml` pattern): `changes` (paths-filter outputs the changed `plugins/<id>`) → `matrix.plugin` → `uses: ./_plugin.yml` (tsc, lint, unit tests, `.vxp` packaging, manifest JSON schema validation, permission declaration check, compatibility check against the SDK version range) → `ci-ok` aggregate.
2. **Signed-index dry run on PRs**: sign with a temporary test key and verify with the matching public key, to validate the index format and the sha256 computation. **The real private key is used only in the release job on main or tags**, stored in `environment: plugin-index` with required reviewers and a branch restriction.
3. **Separate the publish job from the build job**: the build job produces the `.vxp` and sha256 and touches no secrets; the sign-index job only downloads the artifacts, signs and uploads. Same reasoning as the comment in langgraph `release.yml`.
4. **Provenance**: Repo C is public, so it can use `actions/attest-build-provenance` for free to generate an attestation for each `.vxp` (ripgrep/uv). The client can later choose to verify with logic similar to `gh attestation verify`.
5. **Security**: never check out third-party plugin PR code with `pull_request_target` (slint `pr_checks.yaml`); fork PRs run only `pull_request` without secrets; `permissions: {}`; zizmor; actions pinned by SHA. Community plugin PRs can get a read-only metadata check with `pull_request_target` (contributor agreement, changes in the permissions declared in the manifest), as in slint `pr_checks.yaml`.
6. **Schedule**: once a week, rebuild all plugins with the latest published SDK to catch breaking SDK changes early (same idea as slint's scheduled full CI).

### 10.8 Recommended overall diagram
```
                ┌──────────── Repo B (public SDK) ────────────┐
                │ ci.yml → ci-ok    release(tag) → npm OIDC   │
                │ preview(label) → pkg-pr-new                 │
                └───────┬───────────────────────────┬─────────┘
          npm @vestra-player/sdk@x.y.z               │ weekly / dispatch after SDK release
                        ▼                            ▼
┌──────────── Repo A (private) ───────────┐   ┌──── Repo C (public plugins) ────┐
│ ci.yml                                  │   │ ci.yml                           │
│  plan ─▶ static[ubuntu] ─┬▶ windows     │   │  changes ─▶ _plugin.yml × N      │
│                          └▶ macos(cond.)│   │          ─▶ index dry-run sign   │
│                          ─▶ ci-ok       │   │          ─▶ ci-ok                │
│ nightly: audit + full + unsigned pkg    │   │ publish(main): build ─▶ sign     │
│ release(tag): same _static/_build-test  │   │   (env: plugin-index) ─▶ attest  │
│   ─▶ _package ─▶ sign(ubuntu rcodesign  │   │   ─▶ signed index                │
│      / Azure TS) ─▶ draft release       │   └──────────────────────────────────┘
└─────────────────────────────────────────┘
```

### 10.9 Suggested rollout order
1. Repo A: first add `_static.yml` (fmt, TS scripts, SDK diff, cargo-deny licenses), `_build-test.yml` (Windows) and `ci-ok`; **leave macOS out for now** (the local machine is macOS).
2. Add check-unsafe-policy, clippy `-D warnings`, typos, zizmor, actionlint. Make them required after fixing the existing warnings.
3. Add conditional macOS runs and nightly (audit, full, unsigned packages).
4. Add cargo-about to generate third-party license texts, and the check of the non-Cargo dependency list.
5. Release: first only an unsigned draft release, then Windows Azure Trusted Signing, and finally macOS rcodesign signing and notarization (needs a Developer ID certificate and an ASC API key).
6. Repo B: move to npm trusted publishing, add publint, attw, typedoc, and set up the environment. Repo C: add the plugin matrix and the signing dry run, and move the real private key into an environment.

---

### Appendix: list of cited files (all at each repository's HEAD)
- slint-ui/slint: `.github/workflows/{autofix,ci,build_and_test_reusable,build_docs,schedule_ci,notify_ci_failure,schedule_nightly_snapshot,nightly_snapshot,pr_checks,publish_npm_package,visual_editor_macos_dmg,visual_editor_windows_msix}.yaml`, `.github/actions/{setup-rust,codesign}/action.yaml`, `.github/ci_path_filters.yaml`, `scripts/run_clippy.sh`, `.clippy.toml`, `.mise/tasks.toml`
- langchain-ai/langgraph: `.github/workflows/{ci,_lint,_test,_test_release,release,pr_lint}.yml`
- langchain-ai/langgraphjs: `.github/workflows/{ci,release,test-exports}.yml`, `package.json`
- tauri-apps/tauri: `.github/workflows/{fmt,lint-rust,lint-js,audit,supply-chain,udeps,check-license-header,test-core,test-cli-rs,covector-version-or-publish,publish-cli-js,publish-cli-rs}.yml`; tauri-apps/tauri-action: `examples/publish-to-auto-release-universal-macos-app-with-signing-certificate.yml`
- denoland/deno: `.github/workflows/{ci.ts,ci.generated.yml,pr.generated.yml,npm_publish.generated.yml,promote_to_release.generated.yml,start_release.generated.yml}`, `tools/lint.js`
- BurntSushi/ripgrep: `.github/workflows/{ci,release}.yml`
- tokio-rs/tokio: `.github/workflows/{ci,audit,pr-audit}.yml`, `deny.toml`
- astral-sh/uv: `.github/workflows/{ci,plan,check-lint,check-zizmor,check-crates-policies,test,release,sign-release-binaries}.yml`, `Cargo.toml [workspace.lints]`, `.cargo/deny.toml`; astral-sh/github-actions: `sign-macos/action.yml`, `sign-windows/action.yml`
- vitejs/vite: `.github/workflows/{ci,publish,zizmor,semgrep,preview-release}.yml`, `package.json`
