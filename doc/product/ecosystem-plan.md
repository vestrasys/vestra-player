# 插件生态：SDK 独立、插件仓库与插件商城

日期：2026-10-21。本文整理目前讨论的结论，并给出实施步骤。插件商城的细节还没最终定稿；SDK 拆分和仓库拆分已经确定。

## 1. 已定结论

### 1.1 总体方向

- 采用 Dalamud 的模式：不写后端、不用数据库、不做账号系统。
  - 官方仓库托管在 GitHub 上，作者通过 PR 提交插件，由 CI 构建并发布。
  - 第三方可以自建仓库，用户粘贴仓库地址即可使用。
- 客户端只认一种静态 JSON 格式（`repo.json`），不关心它托管在哪里。
- 以后要自建服务器时：
  - 在同一个地址输出同样的 `repo.json`，客户端不用改；
  - 额外能力（服务端搜索、下载统计、更新查询）写在 `repo.json` 的 `extensions` 字段里，客户端发现就用，没有就退回静态行为。
- 作者自助发布、账号、评分都是服务器那一侧的事，到需要时再用 GitHub OAuth 做，不影响客户端协议。

### 1.2 两种元数据分工

| 文件 | 谁写 | 内容 |
| --- | --- | --- |
| 插件包内的 `manifest.json` | 插件作者 | 插件自己的信息：`id`、`version`、`apiVersion`、`capabilities`、`entry`。另新增可选的显示字段：`name`、`author`、`punchline`（一句话简介）、`description`、`tags`、`icon`（包内 PNG/JPEG 路径）、`readme`（默认 `README.md`）、`changelog`、`sourceUrl`、`license` |
| 仓库的 `repo.json` | 由工具或 CI 生成，一般不手写 | 汇总各插件 manifest 的显示字段，再加上只有仓库才知道的信息：每个版本的 `downloadUrl`、`sha256`、`size`、`published`、`channel`（stable/testing），版本历史，`downloadCount`，图标和 README 的对外 URL，仓库名，`official`，`extensions`，签名 |

- 发布到官方仓库的作者只写 `manifest.json`，再在 PR 里附一个 `manifest.toml`，写明源码仓库、commit 和 owners。
- 自建仓库的作者用工具生成 `repo.json`（`vestra-repo build`，或我们提供的 GitHub Actions 模板）。
- 只想发一个包的作者不需要仓库：用户直接粘贴包的链接，或者拖入包文件安装。

### 1.3 `repo.json` 草案（schema 1）

```json
{
  "schema": 1,
  "name": "Vestra Player 官方仓库",
  "official": true,
  "plugins": [
    {
      "id": "vestra.subtitle-online",
      "name": "在线字幕", "author": "…", "punchline": "…", "description": "…", "tags": ["字幕"],
      "iconUrl": "…", "readmeUrl": "…", "changelogUrl": "…", "sourceUrl": "…",
      "downloadCount": 0,
      "versions": [
        { "version": "1.2.0", "apiVersion": "vestra/1", "channel": "stable",
          "downloadUrl": "…", "sha256": "…", "size": 0, "published": "…" }
      ]
    }
  ],
  "extensions": {}
}
```

- 完整性由 `sha256` 保证，与托管方无关。
- 官方仓库的 `repo.json` 由 CI 签名，客户端内置公钥用来校验。
- 第三方仓库在界面上标明「非官方」，并显示插件来自哪个仓库。

### 1.4 客户端（播放器）

- 商城是一个插件，扩展现有的 `plugins` 插件，不进核心。
  - 界面参照 VS Code：左边列表，右边详情。详情页渲染 README，显示下载次数、版本和更新日志。没有图标的插件显示 Vestra Player 的 logo。
  - 用 Node 的 `fetch` 下载，校验 `sha256`，再调用现有的 `plugin.packages.validate(目录).install()` 热安装。热卸载、停用、回退用现有接口。
  - 安装来源：官方仓库、第三方仓库地址（在设置里添加）、插件包链接、拖入插件包文件。
- 核心只需两处小改动：
  1. 内置插件不允许卸载，但可以停用；
  2. 宿主和 SDK 的 manifest 校验放行 1.2 节的显示字段，并检查格式。

  索引签名先由商城插件校验，以后再考虑移进核心。

### 1.5 SDK 独立

- SDK 发布到 npm，所有插件（包括内置插件）都从 npm 包引用：`import type { Plugin } from "@vestra-player/sdk"`。不再使用 `../../sdk/...` 相对路径，插件工坊也不再拷贝 SDK。
- npm 包的内容：
  - 插件 API 类型；
  - `TestHost`（`@vestra-player/sdk/testing`）；
  - 命令行工具 `vestra-plugin`（check / build / test / pack）和 `vestra-repo build`（生成 `repo.json`）；
  - Slint 控件库和主题的只读副本（`@vestra/widgets.slint`、`theme.slint`），供作者在 IDE 里编译和预览。
- 插件模板：`npm create vestra-plugin`。
- 版本对齐：`@vestra-player/sdk@1.x` 对应 `apiVersion: "vestra/1"`。破坏性变更时升大版本。
- **运行时不随 npm 包分发**：
  - 播放器运行时依赖两样东西：Rust 宿主程序，以及 Node.js。每个插件是一个 Node 进程，宿主执行 `node runtime/runner.ts <插件目录>`。
  - runner、SDK 的实现代码和 `wire.ts`（宿主与插件之间的内部协议）都随播放器分发，不发布到 npm，也不对外承诺。
  - 插件构建产物只引用类型，不包含 SDK 代码，所以拆分不改变运行方式。

## 2. 仓库划分（用户将创建）

| 仓库 | 内容 | 发布 |
| --- | --- | --- |
| `vestra-player`（现有） | Rust 宿主、runner、SDK 实现的运行时副本、文档 | 播放器安装包 |
| SDK 仓库 [`vestra-player-sdk`](https://github.com/vestrasys/vestra-player-sdk)（S1 建立） | `@vestra-player/sdk`：类型、TestHost、CLI、`./runtime` 实现入口、Slint 控件库副本、插件模板 | npm（`NPM_TOKEN` 就绪前播放器用 git 标签） |
| 插件仓库（新建） | 官方插件源码，以及第三方插件提交（`manifest.toml`），CI 构建出插件包和 `repo.json` | GitHub Releases（插件包）、GitHub Pages（`repo.json`） |

**SDK 源码的唯一来源**：建议 SDK 仓库作为类型和 CLI 的源头；播放器仓库通过 npm 依赖拿到 SDK 运行时代码，或者在 SDK 仓库里单独导出一个 `runtime` 入口供播放器打包。不要两个仓库各维护一份。具体做法在步骤 S2 里定。**已定（S2）**：SDK 实现也只在 SDK 仓库，经 `./runtime` 入口供播放器使用；播放器仓库删除 `src/packages/sdk`，依赖锁定版本的包，见文末 S1/S2 记录。

**内置插件**：源码迁到插件仓库，播放器打包时从插件仓库的 Releases 取构建好的包，放进安装包。插件更新可以走商城，不必等播放器发版。

## 3. 需要你准备的

1. **GitHub 仓库**：SDK 仓库和插件仓库，告诉我链接。
2. **npm**：
   - 确认包名和作用域，建议 `@vestra-player/sdk`，需要先在 npm 上建组织 `vestra-player`；
   - 发布用的 npm token，配置到 SDK 仓库的 Actions secrets，名字例如 `NPM_TOKEN`。我不直接接触 token。
3. **插件仓库**：启用 GitHub Pages，用来托管 `repo.json`。
4. **签名密钥**：用来给官方 `repo.json` 签名，私钥放进插件仓库的 Actions secrets。可以由我生成密钥对，把私钥交给你配置，公钥写进播放器。

## 4. 实施步骤

| 步骤 | 内容 | 依赖 |
| --- | --- | --- |
| S0 | 在本仓库先把 SDK 整理成可独立的 workspace 包（`src/sdk` 改为包结构，插件改用包名引用，CLI 从 `scripts/plugin.mjs` 移进包），在 monorepo 内验证：全量检查，加开发方式和 dist 两种真机运行 | 无，可立即开始 |
| S1 | SDK 仓库：迁入 S0 的包（保留 git 历史，用 `git subtree split` 或 `git filter-repo`），配置 CI（检查、测试、npm 发布），发布 `@vestra-player/sdk@1.0.0` | SDK 仓库链接、npm 作用域、`NPM_TOKEN` |
| S2 | 播放器仓库改为依赖 npm 包：删除 `src/sdk` 里对外的部分；runner 和运行时代码的来源按第 2 节确定；打包和开发两条路径真机验证 | S1 |
| S3 | 插件仓库：迁入内置插件源码（保留历史），改用 npm 上的 SDK；建 CI，包括构建、打包、上传 Releases、生成并签名 `repo.json`、发布到 Pages、统计下载次数；写 `manifest.toml` 的 PR 模板和贡献说明 | 插件仓库链接、Pages、签名密钥 |
| S4 | 核心改动：manifest 显示字段，内置插件不可卸载；播放器打包时从插件仓库取内置插件 | S2、S3 |
| S5 | 商城插件：列表和详情界面、安装、更新、第三方仓库、链接与拖入安装、签名校验 | S3、S4 |
| S6 | 真机验收：从官方仓库安装和更新、第三方仓库、侧载、热卸载、内置插件保护；跨平台检查 | S5 |

S0 不依赖新仓库，可以先做。拿到仓库链接后从 S1 继续。

## 5. 相关的待办（另行处理）

- macOS `.app` 打包：`Info.plist` 显示名 Vestra Player、应用图标。
- Node.js 运行时：正式版是要求用户自己安装 Node（≥ 22.18），还是随播放器内置一份。和打包一起定。
- Windows 实机运行验证。

## S0 记录（2026-10-21，API 维护者）

SDK 已在本仓库整理成 npm workspace 包 `@vestra-player/sdk@1.0.0`（对应 `vestra/1`），所有插件、示例、runner 和测试都改用包名引用，插件工坊不再带 SDK 副本。插件行为不变：除 workshop 外，全部 18 个插件的 `dist/` 与改动前逐字节相同；workshop 只少了 `vendor/sdk`，`main.js` 随引擎改动变化。

### 包结构

```text
src/packages/sdk/                 npm 包 @vestra-player/sdk（src/package.json 的 workspaces）
  package.json                    exports / bin / files；依赖 typescript ^5.9、@types/node
  README.md                       作者用：安装、最小插件、入口、CLI、Slint 副本
  tsconfig.json                   构建：src → dist（.js + .d.ts）
  tsconfig.check.json             src + test 的类型检查（S1 后包自己的 CI 用）
  scripts/build.mjs               清空并重建 dist/
  src/index.ts                    公共入口：只有类型（export type * from plugin/types.ts）
  src/plugin/*.ts                 插件 API 实现（原 src/sdk/plugin/），testing.ts 为 TestHost
  src/*.ts                        wire 客户端等内部模块（原 src/sdk/*.ts，wire.ts 在内）
  src/player-runtime.ts           ./runtime 入口（仅供播放器）
  src/capabilities.ts             能力名表与 API_VERSION（CLI 与宿主一致性测试用）
  bin/vestra-plugin.mjs           CLI（由 scripts/plugin.mjs 迁入并泛化）
  slint/palette.slint、slint/vestra/{theme,widgets}.slint   控件库只读副本
  test/*.test.ts                  SDK 单元测试（原 src/tests/sdk/ 中只测 SDK 的部分）+ cli.test.ts
```

留在播放器一侧的：`src/runtime/runner.ts`；`src/tests/sdk/`（`runner.test.ts`、`host-agreement.test.ts`：命令上限、能力表、`apiVersion`、Slint 副本与宿主一致）；`src/scripts/plugin.mjs`（按插件名调用 CLI 的快捷方式，`npm run plugin -- …` 用法不变）、`sync-sdk-slint.mjs`、`stage-runtime.mjs`、`check-boundaries.mjs`。

### exports

| 入口 | 文件 | 内容 |
| --- | --- | --- |
| `@vestra-player/sdk` | `dist/index.{js,d.ts}` | 插件 API 全部公共类型，无运行时值（`index.js` 为空模块），所以误写值导入会在构建时被拒 |
| `@vestra-player/sdk/testing` | `dist/plugin/testing.*` | `TestHost`、`startPlugin`、`HostError`、`FakeWindow` 等；新增再导出 `COMMAND_LIMITS`（参考文档原本就把它写成 SDK 常量，原来只能用内部路径取到） |
| `@vestra-player/sdk/runtime` | `dist/player-runtime.*` | `createPlugin`、`PluginInstance`、`PluginManifest`、`HostError`、`Json`、`Transport`、`CAPABILITIES`、`API_VERSION`、`COMMAND_LIMITS`。**仅供播放器内部**（runner 与播放器测试），不属于插件 API，不受 semver 约束 |
| `@vestra-player/sdk/package.json` | | 供 runner 位置解析包目录（工坊用） |
| bin `vestra-plugin` | `bin/vestra-plugin.mjs` | `check / build / test / pack` |

与 §1.5 的差别：SDK 实现代码（含 `wire.ts`）也在 npm 包里，经 `./runtime` 供播放器使用、不对插件承诺。这就是第 2 节「SDK 仓库单独导出一个 `runtime` 入口供播放器打包」的方案，SDK 源码只有一份；runner 仍只随播放器分发。

`createPlugin` 放在 `./runtime` 而不是 `.`：插件只该 `import type`，公共入口没有值就不会被误用。`wire.ts` 等内部模块不在 `exports` 里，Node 和 TypeScript 都拒绝深路径导入；它们的 `.d.ts` 仍在包里，因为公共类型引用了其中的 `Json` 等类型。

### 发布格式：编译后的 .js + .d.ts

- Node 不对 `node_modules` 里的 `.ts` 做类型剥离（`ERR_UNSUPPORTED_NODE_MODULES_TYPE_STRIPPING`）。打包后的 runner 从 `Resources/runtime/node_modules` 加载 SDK，只能是 `.js`；第三方插件的测试也一样。
- 打包器、编辑器、旧版 TypeScript 都直接读 `.js` + `.d.ts`。
- 声明文件保留 `.ts` 后缀的相对引用（TypeScript 的 `rewriteRelativeImportExtensions` 不改 `.d.ts`），已验证 NodeNext 解析正常（类型错误能报出，不会退化成 `any`），`skipLibCheck` 关闭时也无错误。
- 仓库内：插件、示例、runner、播放器测试都用 `dist/`，与第三方作者看到的完全一样；SDK 自己的单元测试通过相对路径直接跑 `src/`。`dist/` 不提交（`.gitignore`），构建约 1 秒：根 `prepare` 在 `npm ci/install` 时构建，`check`、`test:sdk`、`plugin`、`plugins:build`、`start` 先跑 `sdk:build`；包自己用 `prepack`（`npm pack/publish` 前构建）。不用 `prepare` 是因为 npm 11 默认拦截依赖包的安装脚本并告警。
- 包依赖 `typescript ^5.9`（CLI 自带编译器）和 `@types/node`（公共 `.d.ts` 引用 `AbortSignal`、`Buffer`、`node:net`）。不能用宽松的 peer 依赖：npm 上 `typescript` 最新版已是 7.0（原生编译器），它不导出 `lib/tsc.js` 和 `transpileModule`，CLI 用不了。运行时没有任何依赖（只用 Node 内置模块）。
- `license` 暂填 `UNLICENSED`：**S1 发布前需要用户定许可证**。

### CLI `vestra-plugin`

- 按目录工作（默认当前目录），不再假设仓库路径：类型检查沿用插件目录或上层最近的 `tsconfig.json`（到 `.git` 所在的仓库根为止；本仓库即 `src/tsconfig.json`，`jsnes` 路径映射照旧生效），没有就用内置严格设置；`typeRoots` 收集插件目录往上的各级 `node_modules/@types`，再加 SDK 自带的 `@types/node`。
- 新检查：manifest 的 `apiVersion` 必须是 `vestra/1`（宿主本来就拒绝其它值）。产物中对 `@vestra-player/sdk`（任何子路径）的运行时引用一律拒绝，改为解析语法树判断，不再用正则（工坊引擎的字符串里含 `from '@vestra-player/sdk'`，正则会误报）；打包（`bundle`）时把 SDK 标为 external，确保这项检查看得到。
- `pack`：构建后把 `dist/` 打成 `<id>-<version>.zip`（条目排序、固定时间戳，同样输入得到同样字节），输出大小和 sha256，供以后的 `repo.json`。**这是暂定格式**：宿主现在只从目录安装，包格式在 S3/S5 定稿时再确认。`*.zip` 不会被复制进 `dist/`（需要时列入 `assets`）。
- Windows：`npm ci` 通过 shell 调用（`npm.cmd`）。

### 边界检查（check-boundaries）

插件和示例只能引用：自己目录内的相对路径；`@vestra-player/sdk`；`tests/` 下另可引用 `@vestra-player/sdk/testing`。`/runtime`、深路径、指向 SDK 源码的相对路径都报错（已用临时文件验证两种违规会失败）。

### 插件工坊

删除 `vendor/sdk` 和 `tests/vendor-sdk.test.mjs`。生成时用 `playerSdk()`：从 `process.argv[1]`（播放器的 runner）按 Node 规则解析 `@vestra-player/sdk/package.json`，开发时得到 workspace 包，打包后得到 `Resources/runtime/node_modules/@vestra-player/sdk`；只把 `package.json` 和 `dist/**/*.d.ts` 复制进候选的 `node_modules/@vestra-player/sdk/`。生成的 `main.ts` 与普通插件一样写 `import type { Plugin } from '@vestra-player/sdk'`。新增 `tests/sdk-types.test.mjs`，`engine.test.mjs` 增加断言（候选只含类型文件、导入写法），17/17 通过。

### Slint 控件库副本

`src/scripts/sync-sdk-slint.mjs` 把 `host/ui/palette.slint`、`host/ui/vestra/{theme,widgets}.slint` 复制到包的 `slint/`（`theme.slint` 引用 `../palette.slint`，所以连同调色板一起复制，相对位置不变）；`--check` 只报告差异。`tests/sdk/host-agreement.test.ts` 要求副本逐字节一致，且 `host/ui/vestra/` 里每个文件都在同步列表中。作者在 Slint LSP 里把库路径 `vestra` 指到 `node_modules/@vestra-player/sdk/slint/vestra`；运行时以播放器自带的为准。

### runner 如何解析 SDK

`runtime/runner.ts` 改为 `import { createPlugin, HostError } from "@vestra-player/sdk/runtime"`，宿主启动方式和 runner 路径都不变（开发：`src/runtime/runner.ts`；打包：`Resources/runtime/runner.ts`），Node 从 runner 所在目录往上找 `node_modules`：

- 开发：`src/node_modules/@vestra-player/sdk` 是 npm workspace 建的符号链接，指向 `src/packages/sdk`，加载 `dist/`。直接 `cargo run` 时用的是最近一次构建的 `dist/`：改了 SDK 源码要先 `npm run sdk:build`（或 `npm start`）。
- 打包：`Resources/runtime/node_modules/@vestra-player/sdk`（复制，不是链接）。

### 打包时 Resources/runtime 的内容

```text
Resources/runtime/
  runner.ts                                  src/runtime/runner.ts
  package.json                               {"type":"module","private":true}
  node_modules/@vestra-player/sdk/
    package.json                             exports 表（runner 和工坊都靠它解析）
    dist/**/*.js                             SDK 实现
    dist/**/*.d.ts                           工坊给生成的候选做类型检查要用
```

不需要 `npm install`（SDK 没有运行时依赖），也不需要 `bin/`、`slint/`、TypeScript。`node src/scripts/stage-runtime.mjs <目录>` 按这个布局生成（先 `npm run sdk:build`）；`tests/sdk/runner.test.ts` 新增一条：在仓库外的临时目录生成该布局，用其中的 runner 跑一个插件到 `runtime.ready`。S2 改为依赖 npm 上的包后，`node_modules/@vestra-player/sdk` 换成从 npm 包解出的同样文件即可。

### 检查（本次实际执行）

- `src/`：`npm ci`（干净安装，`prepare` 生成 `dist/`）、`check`、`format:check`、`boundaries` 均退出 0；`test:sdk`：包内 105/105（原 103 + CLI 2），播放器侧 9/9（`runner` 6 + 打包布局 1，`host-agreement` 3）。改动前基线 109/109。
- `plugins:build` 退出 0；各插件与示例测试通过数与改动前完全相同：audio 35、bilibili 15、capture 25、history 22、hotkeys 17、info 31、marks 24、nes 11、playback 64、playlist 98、plugins 10、preferences 10、subtitle-online 17、subtitles 52、video 39、visualizer 17、window 34、workshop 17、youtube 14、examples bookmark 1 / core-ui 2 / panel-menu 1；三个示例 `check` 通过。
- `npm pack`：76 个文件（`README.md`、`package.json`、`bin/`、`dist/`、`slint/`），没有 `src/`、`test/`、`scripts/`，`dist` 里没有指向仓库路径的引用。在仓库外的临时目录用 tarball 新建最小插件项目（`npm i -D <tarball>`，自动带上 TypeScript 5.9.3 和 `@types/node`），`vestra-plugin check / build / test / pack` 全部通过；构建产物只有 `main.js`、`manifest.json`、`package.json`；故意写错类型、写成值导入时 `check` / `build` 按预期失败。
- `src/host/`（`CARGO_TARGET_DIR=~/.cache/vestra-target/s0`）：`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings` 通过，`cargo test --locked` 263/263。Rust 只改了两处注释。
- `portability`：clean。宿主可编译到 Windows 目标；扫描范围改为 `plugins`、`packages/sdk/{src,bin,scripts}`、`runtime`，0 处系统命令；`process.platform` 分支 4 处（新增 1 处是 CLI 在 Windows 上经 shell 调用 `npm`）。

### 给协调者的真机验证要点

1. **开发方式**：`cd src && npm ci`（确认 `packages/sdk/dist/` 已生成）→ 在 `src/host/` 下 `cargo run --bin vestra-host -- --plugin ../plugins/<每个插件>`，全部插件激活、菜单齐全。确认插件进程报错里没有 `Cannot find package '@vestra-player/sdk'`。
2. **dist 方式**：`npm run plugins:build` 后加载各插件的 `dist/` 目录，同上。
3. **打包布局**：用 `node scripts/stage-runtime.mjs` 生成 `Resources/runtime` 并放进 `.app`（或任何 `<exe 上一级>/Resources/runtime`），确认宿主走打包路径的 runner、插件正常激活。
4. **插件工坊**（开发方式和打包布局各一次）：生成 → 校验 → 试载 → 安装 → 回退。重点：候选目录现在多了 `node_modules/@vestra-player/sdk/`（约 40 个 `.d.ts`，没有符号链接），确认宿主 `packages.validate` 接受；打包布局下 TypeScript 诊断仍是真实的 `TS2322`，说明找到了 `Resources/runtime` 里的 SDK 类型。
5. **Windows**：`npm ci` 能生成 `dist/`；`npm run plugin -- test <名字>` 对有依赖的插件（workshop、nes）会经 shell 调用 `npm ci`；`packages/sdk/test/cli.test.ts` 用目录联接（junction），需要在 Windows 上跑一次。

### 给 S1 的备注

- 迁到 `vestra-player-sdk` 仓库的就是 `src/packages/sdk/` 整个目录（它的 `package.json`、`tsconfig*.json`、`test/` 已能独立运行：`npm run check`、`npm test`、`npm pack`）。保留历史：`git filter-repo --path src/sdk --path src/tests/sdk --path src/packages/sdk --path-rename src/packages/sdk/:` 之类，S0 的第一个提交只做了改名，便于追溯。`src/tests/sdk/runner.test.ts` 是播放器侧的，不迁。
- Slint 副本的同步方向不变（播放器是源头）：SDK 仓库发布时从对应的播放器版本复制，播放器侧的一致性测试在 S2 改为对比 npm 包里的副本。
- 发布前要定：许可证、npm 组织 `vestra-player`、`NPM_TOKEN`（第 3 节）。

## S1/S2 记录（2026-10-21，API 维护者）

SDK 源码迁到独立仓库 `vestrasys/vestra-player-sdk`（私有），保留历史；播放器仓库删除 `src/packages/sdk`，改为依赖 SDK 仓库的 `v1.0.0` 标签。SDK 源码只有一份。npm 组织和 `NPM_TOKEN` 还没就绪，**这次没有发布到 npm**，过渡期用 git 依赖。

### S1：SDK 仓库

- **历史**：从播放器仓库 `git filter-repo` 只保留三段路径并改名到仓库根：`src/sdk/` → `src/`，`src/packages/sdk/` → `./`，SDK 自己的 14 个单元测试 `src/tests/sdk/<名>.test.ts` → `test/`（`runner.test.ts`、`host-agreement.test.ts` 留在播放器）。新旧布局一致，`git log` 不加 `--follow` 也连续：例如 `src/wire.ts` 18 个提交，最早到 2026-10-19 的 `22082b3`（原 `src/sdk/wire.ts`）；`test/round2.test.ts` 6 个提交（原 `src/tests/sdk/round2.test.ts`）。其它分支和标签都已删除，只有 `main`。S0 的纯改名提交在新路径下为空，被 filter-repo 去掉。
- **补齐**：`.gitignore`（`dist/`、`node_modules/`）、`package-lock.json`、`repository`、`publishConfig.access: public`、Prettier 与 `format`/`format:check`；README 增加版本与 `apiVersion` 对应表、git 标签安装、开发与发版步骤、Slint 副本来源。`license` 仍是 `UNLICENSED`，等用户定。
- **构建改用 `prepare`**（替代 `prepack`），理由见下面「依赖方案」。`npm ci`、`npm pack`、`npm publish` 和 git 安装都会运行它。
- **CI**：`.github/workflows/ci.yml`（push 到 `main`、PR；macOS/Linux/Windows × Node 22.18/24）：`npm ci`、`check`、`format:check`、`build`、`test`、`npm pack --dry-run`。Windows 行顺带覆盖 S0 提到的 `cli.test.ts` 目录联接。
- **发布**：`.github/workflows/publish.yml`，推送 `v*` 标签时运行：没有 `secrets.NPM_TOKEN` 就报错退出；标签与 `package.json` 的 `version` 不一致也退出；然后 `npm ci`、`check`、`test`、`npm publish --access public`。不加 `--provenance`：npm 不给私有仓库生成来源证明，仓库公开后可以加上（同时给 `id-token: write`）。
- 提交 `SDK split S1: standalone repository for @vestra-player/sdk`，带注解的标签 `v1.0.0`。

### S2：播放器依赖 SDK 包

`src/package.json`：

```json
"dependencies": { "@vestra-player/sdk": "git+https://github.com/vestrasys/vestra-player-sdk.git#v1.0.0" },
"allowScripts": { "github:vestrasys/vestra-player-sdk": true }
```

- 去掉 `workspaces`、根 `prepare` 和 `sdk:build`；`check`、`plugin`、`plugins:build`、`start` 不再先构建 SDK。`test:sdk` 只跑播放器侧的 `tests/sdk/`（runner、打包布局、宿主一致性）；SDK 单元测试在 SDK 仓库的 CI 里跑。`tsconfig.json`、`format` 不再包含 SDK 路径。
- `package-lock.json` 记录 `resolved: git+ssh://git@github.com/vestrasys/vestra-player-sdk.git#73961229…`（`v1.0.0` 指向的提交）。这是真实 git 安装生成的：用 `GIT_CONFIG_*` 的 `url.<本地 SDK 仓库>.insteadOf` 把 GitHub 地址临时指到本地仓库，npm 走的是和远端完全相同的流程（解析标签、克隆、安装依赖、`prepare`、打包）。推送后远端的标签若仍指向同一提交，lock 无需改动；保险起见推送后再 `npm install` 一次。
- 访问私有仓库：npm 先试 codeload 的 tarball（私有仓库 404），再 `git clone https://…`（走本机 git 凭据 helper，已配置 gh），失败再试 ssh。CI 需要能读 SDK 仓库的凭据（以后改成 npm 依赖后就不需要）。
- 改到播放器一侧的跨仓库检查：
  - `scripts/sync-sdk-slint.mjs`：默认校验**已安装** SDK 包里的 `slint/` 与 `host/ui` 逐字节一致；`--write <SDK 仓库目录>` 把 `host/ui` 复制进 SDK 仓库的 `slint/`。副本存放在 SDK 仓库，内容的源头仍是宿主。流程写进 [工作流 §1.6](../process/community-workflow.md)。
  - `tests/sdk/host-agreement.test.ts`：能力表、`apiVersion`、命令上限（从已安装包的 `./runtime` 读取）与宿主 Rust 源码一致，再跑上面的 Slint 校验。
  - `scripts/stage-runtime.mjs`：从 `node_modules/@vestra-player/sdk` 复制 `package.json` 和 `dist/`（布局不变）。
  - `scripts/check-portability.mjs`：扫描已安装包的 `dist/`、`bin/`（原来扫 `packages/sdk/{src,bin,scripts}`）。
- 插件、示例、runner、插件工坊、`scripts/plugin.mjs` 都按包名解析，代码不变。开发方式下 `src/node_modules/@vestra-player/sdk` 现在是安装出的普通目录，不再是指向源码的链接；要试未发版的 SDK，按工作流 §1.6 临时 `npm install <SDK 仓库目录>`。
- 宿主 Rust 只改了两处注释（SDK 文件位置）。

### 依赖方案：git 标签 + `prepare`

npm 安装 git 依赖时会克隆仓库，若有 `prepare`（或 `build`、`prepack` 等）就在克隆里装依赖，**但之后只运行 `prepare`**，再按 `files` 打包。S0 的包只有 `prepack`，git 安装会得到没有 `dist/` 的包，所以 S1 改用 `prepare`。

- npm 11 的安装脚本策略（`allowScripts`）把 git 依赖的 `prepare` 算作安装脚本：未登记时只告警（`--strict-allow-scripts` 时报错）。播放器的 `package.json` 用 `allowScripts` 按仓库登记了这个包，`npm ci --strict-allow-scripts` 也通过。以后改成 npm 依赖时这一项可以删掉（从 registry 安装不运行 `prepare`）。
- 比较过的另一种方案：在 SDK 仓库为每个版本做一个带 `dist/` 的发布标签。它不需要安装时构建，但同一版本会有两个提交、源码标签和安装标签不一致，还要多一套发版步骤。`prepare` 方案只有一个标签，安装结果与 `npm pack` 完全相同（已逐文件比较 `dist/`），所以选它。安装时的构建只用包自己的依赖（TypeScript 5.9），约 1 秒。

### 推送后需要做的（协调者，按顺序）

1. SDK 仓库：`git -C <SDK 仓库> remote add origin https://github.com/vestrasys/vestra-player-sdk.git`，`git push -u origin main`，`git push origin v1.0.0`。
   - 推送 `v1.0.0` 会触发 `publish.yml`，因为还没有 `NPM_TOKEN` 会**按预期失败**（「Secret NPM_TOKEN is not set」）。不想看到失败的话，先推 `main`、等设好 token 再推标签——但播放器在那之前装不到 `v1.0.0`，所以建议照常推标签、忽略这次失败。
   - 确认 `CI` 工作流在三个系统上通过（Windows 行是 `cli.test.ts` 目录联接的首次实测）。
2. 播放器 `rebuild/sdk-repo` 分支：`cd src && rm -rf node_modules && npm ci`，确认 `node_modules/@vestra-player/sdk/dist/` 存在、没有安装脚本错误；若 npm 改写了 `package-lock.json`（远端标签提交与本地不同），提交更新后的 lock。
3. 跑 §6.1 全部检查，然后按 S0 记录「给协调者的真机验证要点」做开发方式、dist 方式、打包布局、插件工坊的真机验证。
4. 合并 `rebuild/sdk-repo` 到 `main` 并推送。
5. 以后（npm 就绪）：建组织 `vestra-player`、配置 `NPM_TOKEN`、定许可证，在 SDK 仓库重新运行 `v1.0.0` 的 Publish（或发 `v1.0.1`），播放器改为 `"@vestra-player/sdk": "1.0.0"`、删掉 `allowScripts`，`npm install` 刷新 lock。

### 检查（本次实际执行）

- SDK 仓库：`npm ci`（`prepare` 生成 `dist/`）、`check`、`format:check`、`build` 退出 0；`npm test` 105/105；`npm pack`：76 个文件（`README.md`、`package.json`、`bin/`、`dist/`、`slint/`），与 S0 相同。在临时项目里 `npm install "git+file://<SDK 仓库>#v1.0.0"` 得到带 `dist/` 的包；不登记 `allowScripts` 时 `--strict-allow-scripts` 报错，登记后通过。
- 播放器（`rebuild/sdk-repo` worktree，全新 `node_modules`）：`npm install` / `npm ci --strict-allow-scripts`（GitHub 地址经 `insteadOf` 指到本地 SDK 仓库），安装出的 `dist/` 与 SDK 仓库构建的逐文件相同；`check`、`format:check`、`boundaries` 退出 0；`test:sdk` 9/9（`runner.test.ts` 6，含打包布局；`host-agreement.test.ts` 3）；`sdk:sync-slint` 一致，人为改动已安装副本后校验与 `host-agreement` 按预期失败，`--write` 写出 3 个文件。
- `plugins:build` 退出 0；插件与示例测试与 S0 完全相同：audio 35、bilibili 15、capture 25、history 22、hotkeys 17、info 31、marks 24、nes 11、playback 64、playlist 98、plugins 10、preferences 10、subtitle-online 17、subtitles 52、video 39、visualizer 17、window 34、workshop 17、youtube 14、examples bookmark 1 / core-ui 2 / panel-menu 1；三个示例 `check` 通过。（全新 worktree 里 `check` 要在 `plugins:build` 之后跑：`plugins/nes` 的 `jsnes` 由 CLI 的 `npm ci` 安装，与 S0 相同。）
- `src/host/`（`CARGO_TARGET_DIR=~/.cache/vestra-target/s1`）：`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings` 通过，`cargo test --locked` 263/263。
- `portability`：clean；扫描已安装 SDK 的 `dist/`、`bin/`，0 处系统命令，`process.platform` 分支 4 处（与 S0 相同）。

