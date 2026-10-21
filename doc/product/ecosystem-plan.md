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
| SDK 仓库（新建） | `@vestra-player/sdk`：类型、TestHost、CLI、Slint 控件库副本、插件模板 | npm |
| 插件仓库（新建） | 官方插件源码，以及第三方插件提交（`manifest.toml`），CI 构建出插件包和 `repo.json` | GitHub Releases（插件包）、GitHub Pages（`repo.json`） |

**SDK 源码的唯一来源**：建议 SDK 仓库作为类型和 CLI 的源头；播放器仓库通过 npm 依赖拿到 SDK 运行时代码，或者在 SDK 仓库里单独导出一个 `runtime` 入口供播放器打包。不要两个仓库各维护一份。具体做法在步骤 S2 里定。

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
