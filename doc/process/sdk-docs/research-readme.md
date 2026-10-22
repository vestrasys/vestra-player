# @vestra-player/sdk README / CONTRIBUTING 改写调研

日期：2026-10-22。只读调研，没有改动任何仓库文件。原始 README 抓取到 `scratchpad/readmes/`。

---

## 0. 现状

**SDK 仓库**（`scratchpad/sdkrepo`，1.2.0）

- `README.md`：73 行，纯文字。没有 logo，没有徽章，也没有截图。内容依次是一句话介绍、API 文档链接、安装（附一段 Node 26 的解释）、最小插件、CLI 用法、入口点表、版本表、License，最后是写给维护者的 Pages/CI 说明。
  - 问题一：开头没有 hero 区，读者看不出这是什么产品的 SDK。现在只链到 `github.com/vestrasys` 组织页，没有播放器介绍，也没有截图。
  - 问题二：三类读者混在一页里。插件作者要看安装和快速开始，深入使用的人要看入口点和版本矩阵，维护者要看 Pages 和 CI。
  - 问题三：`Versions` 表里 1.1 那一格塞了一长串 API 名，属于 CHANGELOG 的内容。
  - 问题四：README 同时被 TypeDoc 当作文档站首页（`typedoc.json` 里的 `"readme": "README.md"`），所以任何 HTML 写法都要能在 GitHub、npm、TypeDoc 三处正常显示。
- `CONTRIBUTING.md`：85 行，标题是 "Maintaining @vestra-player/sdk"，全部是写给维护者的内部手册：构建命令、TypeDoc 规则、CI 拓扑图、发布流程、npm Trusted Publishing 的一次性设置、与播放器的同步。外部贡献者需要的内容一项都没有：如何提 issue、PR 规则、提交规范、行为准则、许可声明。
- `docs/getting-started.md`：完整的入门指南。**发现一处过期内容**：第 12 行还写着 "Node 22.18 or newer"，而 package.json 的 engines 和 README 都已经是 Node 26。
- 没有 `examples/` 目录。CLI 只有 `check / test / build / pack`，没有 `init` 或脚手架。
- Workflow 文件名：`ci.yml`（name: CI，非 main 分支和 PR 触发）、`main.yml`（name: Main，push 到 main 时触发并部署 Pages）、`release.yml`。

**Logo**：`/Users/ziyang/Develop/vestra-player/src/assets/brand/logo.png`。1024×1024 的 RGBA PNG，368 KB，一个橙→粉→紫→蓝渐变的圆角 "V"。渐变色本身在深色和浅色背景上都够醒目，**所以不需要准备深色、浅色两套 logo**。前提是背景确实透明，这一点我只从 RGBA 格式推断，没有逐像素验证，复制时请顺手确认。SDK 仓库里目前没有任何 logo 文件。

**示例**：播放器仓库 `src/examples/` 里有 4 个：`bookmark`（窗口、事件、播放控制）、`core-ui`（mount、osd、主题）、`markdown`（ui.window 和 markdown，manifest 带中文显示字段和 icon）、`panel-menu`（ui.mount 面板）。每个都有 README、manifest、main.ts、tests/，形态完整，可以直接作为 SDK 仓库 `examples/` 的来源。

---

## 1. 逐仓库笔记

| # | 仓库 | 原文 |
|---|---|---|
| 1 | vitejs/vite | <https://raw.githubusercontent.com/vitejs/vite/main/README.md>，npm 版 <https://raw.githubusercontent.com/vitejs/vite/main/packages/vite/README.md> |
| 2 | slint-ui/slint | <https://raw.githubusercontent.com/slint-ui/slint/master/README.md> |
| 3 | tauri-apps/tauri | <https://raw.githubusercontent.com/tauri-apps/tauri/dev/README.md> |
| 4 | astral-sh/uv | <https://raw.githubusercontent.com/astral-sh/uv/main/README.md> |
| 5 | oven-sh/bun | <https://raw.githubusercontent.com/oven-sh/bun/main/README.md> |
| 6 | denoland/deno | <https://raw.githubusercontent.com/denoland/deno/main/README.md> |
| 7 | figma/plugin-typings | <https://raw.githubusercontent.com/figma/plugin-typings/master/README.md> |
| 8 | obsidianmd/obsidian-api 与 obsidian-sample-plugin | <https://raw.githubusercontent.com/obsidianmd/obsidian-api/master/README.md>、<https://raw.githubusercontent.com/obsidianmd/obsidian-sample-plugin/master/README.md> |
| 9 | microsoft/vscode-extension-samples 与 vscode-generator-code | <https://raw.githubusercontent.com/microsoft/vscode-extension-samples/main/README.md>、<https://raw.githubusercontent.com/microsoft/vscode-generator-code/main/README.md> |
| 10 | raycast/extensions | <https://raw.githubusercontent.com/raycast/extensions/main/README.md> |
| 11 | mpv-player/mpv | <https://raw.githubusercontent.com/mpv-player/mpv/master/README.md> |
| 12 | obsproject/obs-plugintemplate | <https://raw.githubusercontent.com/obsproject/obs-plugintemplate/master/README.md> |
| 附 | tauri-apps/plugins-workspace | <https://raw.githubusercontent.com/tauri-apps/plugins-workspace/v2/README.md> |

**1. Vite（66 行）。** 最值得模仿的 hero 区。
- 外层是 `<p align="center">`，里面用 `<picture>` 放深色、浅色两套 logo，高 60px。
- 下面一行 4 个居中徽章：npm 版本 `img.shields.io/npm/v/vite`、Node 版本 `img.shields.io/node/v/vite`、GitHub 原生 CI 徽章 `actions/workflows/ci.yml/badge.svg?branch=main`、Discord。
- 然后是 H1 加 emoji，一行引用块写 tagline，6 条带 emoji 的要点，两段介绍，最后是 "Read the Docs" 链接。
- 再往下是 Packages 表（每行带一个 npm 版本徽章）、Contribution 一行、License 一行、赞助商。
- **关键点：npm 上用的是另一份 README**（`packages/vite/README.md`，20 行）。它没有 logo，没有徽章，也没有 HTML，只有标题、tagline、要点和文档链接。
- README 里没有安装命令，安装和用法全部交给文档站。

**2. Slint（约 240 行）。**
- 开头用 `![](logo-light.svg#gh-light-mode-only) ![](logo-dark.svg#gh-dark-mode-only)` 切换深浅色 logo。这是旧写法，GitHub 已不推荐，在 npm 上会两张图都显示。
- 3 个徽章：CI、REUSE、Discussions。
- 加粗的一句话定位，再一句说明用法。
- "Why Slint" 写成要点。Examples 用表格平铺截图和视频（Windows/macOS/Linux 截图、WASM demo），链到 `examples/`。
- Hello World 是一段 10 来行代码，后面接文档链接，再往后是较长的架构介绍、License、FAQ、公司介绍。
- 图片链接全部用引用式 `[#4]: https://...`，表格源码因此比较整洁。

**3. Tauri（96 行）。**
- 开头是一整张 splash 图（`.github/splash.png`），不用单独的 logo。
- 8 个徽章（status、license、CI、FOSSA、Discord、website 等），偏多。
- Getting Started 只有一行 `npm create tauri-app@latest`，并链到文档。
- Features 是要点，平台支持用表格。Contributing 段落链到 `.github/CONTRIBUTING.md`。

**4. uv（326 行）。**
- 标题下面是 3 个徽章（PyPI 版本、Python 版本、Discord）、一句 tagline，再用 `<picture>` 放深色、浅色两版 benchmark 图。
- Highlights 是要点，每条都链到文档。Installation 给出多种安装方式，每种都在一个小代码块里。
- Features 每个子命令配 5 到 10 行示例。后面是 Contributing 一段、FAQ、License。
- 偏长，但每节都短，扫读成本低。

**5. Bun（446 行）。**
- 居中 logo（170px），H1 也居中。3 个徽章：Discord、stars、"speed: fast"。
- 下面一行居中导航：Documentation • Discord • Issues • Roadmap，后面接 "Read the docs →"。
- "What is Bun" 用短代码块演示核心命令，然后是 Install。
- 后面用 200 多行列出 Quick links 和 Guides 的目录，这部分是纯链接墙，不建议学。

**6. Deno（116 行）。**
- 徽章用引用式链接集中放在文件末尾，正文因此干净。
- logo 用 `<img align="right" height=150>` 浮在介绍段右侧，是一种不占首屏高度的做法。
- 结构依次是：Installation（各平台一行命令）、"Your first Deno program"（3 行代码加 1 行运行命令）、Additional resources（4 条加粗链接）、Contributing 一行。这是**"是什么、装、跑起来、去哪看更多"的标准范例**。

**7. Figma plugin-typings（33 行）。** 和我们最像：一个"宿主 API 类型包"。
- 标题加一个 npm 徽章，一句介绍。
- Usage 分两步编号（安装、tsconfig 配置），About 一段讲版本对应关系。
- 极简，但缺少到插件文档的醒目入口。

**8. Obsidian。**
- `obsidian-api` 的 README（69 行）是纯文字：文档链接、模板链接、issue 去论坛，然后用要点讲 manifest 字段和 App 架构，没有徽章也没有 logo。
- `obsidian-sample-plugin` 是 **GitHub Template 仓库**："Use this template"、clone、`npm i`、`npm run dev`，再加发版和上架流程。
- 也就是说：**类型包只负责"是什么、去哪看文档"，上手流程放在模板仓库里**。

**9. VS Code。**
- `vscode-extension-samples` 的标题是 `<h1 align="center">`，后面一段说明，并承诺每个 sample 都有说明、GIF 或截图、指南链接和所用 API 列表。
- 正文先列 "Getting Started" 的 4 个 hello-world 变体，然后是一张大表：Sample | 官网指南 | 用到的 API。表格由脚本在 `<!-- SAMPLES_BEGIN -->` 与 `<!-- SAMPLES_END -->` 之间自动生成。
- 每个 sample 是独立的顶层目录，名字都叫 `*-sample`。
- `vscode-generator-code`（yo code）有 4 个徽章：npm 版本、月下载量、CI、License。用法写成"安装、`yo code`、截图"，后面是 CLI 选项的完整文本块。

**10. Raycast extensions（31 行）。**
- 居中 logo（128px）和居中 H1，两个 `style=for-the-badge` 的大徽章（X、Slack），一句介绍，一张 header 大图。
- Getting Started 只链到 developers.raycast.com。CONTRIBUTING 只有 10 行，链到文档站的 "Create your first extension" 和 "Contribute to an extension"。

**11. mpv（222 行）。** 代表"老派"写法。
- 开头 128px logo，下面是目录，外链列表（Wiki、User Scripts、FAQ、Manual）。
- 正文是 Overview、系统要求、编译、发布周期、Bug 报告、Contributing、License。
- 没有徽章，偏向用户和打包者。对 SDK 参考价值低，只能借鉴"logo + 外链列表"的克制做法。

**12. OBS plugin template（59 行）。**
- 模板仓库，没有 logo 和徽章，用表格列出构建环境，Quick Start 链到 wiki。
- 其余篇幅讲 CI 和签名，偏维护者视角。可以借鉴"README 只指路，详情放 wiki 或文档"。

**附：tauri plugins-workspace。** 用一张"插件名 | 一句话说明"的表把所有插件列出来，每行链到子目录。这是 examples/ 索引的好形态。

### CONTRIBUTING 原文

| 仓库 | 原文 | 结构和要点 |
|---|---|---|
| Vite | <https://raw.githubusercontent.com/vitejs/vite/main/CONTRIBUTING.md> | 382 行。依次是 Repo Setup、Documentation、依赖原则（"添加依赖前先想想"）、"别再加新选项"、Debugging、Running Tests、**AI Policy**、**Pull Request Guidelines**（新功能要先开 issue、要配测试；bug 修复在标题里写 `fix #123`；不接受纯风格重构；提交时自动格式化；PR 标题遵循 commit convention）、Maintenance（issue 分诊、PR 评审）、Release。 |
| Tauri | <https://raw.githubusercontent.com/tauri-apps/tauri/dev/.github/CONTRIBUTING.md> | 120 行。开头先引用 Code of Conduct，并附一个小目录。正文是 Issue Reporting（必须给复现步骤；"need repro" 5 天无回复即关闭）、PR Guidelines（commit 必须签名；squash 合并；新功能先开 issue）、Development Guide、AI Tool Policy、Financial Contribution。 |
| Slint | <https://raw.githubusercontent.com/slint-ui/slint/master/CONTRIBUTING.md> | 63 行，**最适合我们参考的规模**。依次是欢迎语和 Discussions 链接、good first issue 标签、AGENTS.md 提示；Pull Requests（小而专注；大功能先讨论；公共 API 变更要先和团队商量，因为 1.x API 稳定）；指向内部文档的链接（development、building、testing、triage、写作风格）；License（贡献按 MIT-0 授权，首个 PR 由机器人勾选确认）；Coding Style（CI 强制 rustfmt 和 clang-format）。**细节放到 docs/ 下的子文档，CONTRIBUTING 本身只负责导航。** |
| uv | <https://raw.githubusercontent.com/astral-sh/uv/main/CONTRIBUTING.md> | 387 行。依次是 Finding ways to help、Use of AI、Setup、Testing（含快照测试）、Formatting、Linting、Crate structure、Profiling、Documentation、macOS 代码签名、Releases。 |
| Bun | <https://raw.githubusercontent.com/oven-sh/bun/main/CONTRIBUTING.md> | 375 行，几乎全是构建环境搭建和排错。 |
| mpv | <https://raw.githubusercontent.com/mpv-player/mpv/master/DOCS/contribute.md> | 依次是联系方式、Reporting issues（"报告你观察到的现象，而不是你的推断"）、Sending patches（PR 描述要短；一个 PR 只做一件事）、版权和许可声明、代码风格。 |
| Raycast | <https://raw.githubusercontent.com/raycast/extensions/main/CONTRIBUTING.md> | 10 行，全部链到文档站。 |

---

## 2. 模式对照表

| 仓库 | Logo | 徽章（数量 / 类型） | 长度 | 快速开始形态 | 截图 / GIF | 语言切换 | 示例组织 | 脚手架 |
|---|---|---|---|---|---|---|---|---|
| Vite | 居中，`<picture>` 深浅两套，60px | 4：npm、node、CI、Discord | 66 行（npm 版 20 行） | 不放，直链文档 | 无 | 无（文档站另有翻译） | monorepo packages 表 | `create-vite` |
| Slint | `#gh-*-mode-only` 两套 | 3：CI、REUSE、Discussions | 约 240 行 | Hello World 约 10 行 | 截图表格 | 无 | `examples/` 加截图表 | C++/Rust 模板仓库 |
| Tauri | 整幅 splash 图 | 8（偏多） | 96 行 | 一行 `npm create tauri-app` | splash 图 | 无 | 独立的 plugins-workspace | `create-tauri-app` |
| uv | 无 logo，用 `<picture>` benchmark 图 | 3：PyPI、pyversions、Discord | 326 行 | 每个功能 5 到 10 行 | benchmark 图 | 无 | 无 | — |
| Bun | 居中 170px | 3：Discord、stars、speed | 446 行 | 一组命令 | 无 | 无 | 文档 guides | `bun init` |
| Deno | `align=right` 浮动 | 5 个社交类（引用式） | 116 行 | 3 行代码加 1 行运行 | 无 | 无 | 文档 | `deno init` |
| Figma typings | 无 | 1：npm | 33 行 | 安装加 tsconfig 两步 | 无 | 无 | 无 | Figma 桌面端生成 |
| Obsidian api | 无 | 0 | 69 行 | 无，链到模板 | 无 | 无 | sample-plugin 模板仓库 | GitHub Template |
| VS Code samples | `<h1 align=center>` 无图 | 0（yo code 有 4 个） | 95 行 | clone、`npm i`、F5 | 每个 sample 自带 GIF | 无 | 顶层 `*-sample/` 加自动生成的表 | `yo code` |
| Raycast | 居中 128px 加 header 大图 | 2 个社交类，for-the-badge 风格 | 31 行 | 链到文档 | header 图 | 无 | 仓库即扩展集合 | `npm init raycast-extension` |
| mpv | 128px 左对齐 | 0 | 222 行 | 编译说明 | 无 | 无 | wiki | — |
| OBS template | 无 | 0 | 59 行 | 链到 wiki | 无 | 无 | 模板自身 | GitHub Template |

**结论**

1. **Hero 区的标准格式**：居中 logo（60 到 170px）、一行 3 到 5 个徽章、一句粗体或引用块写的 tagline、一行导航链接（Docs • Examples • Changelog）。在调研的仓库里，徽章超过 5 个的只有 Tauri，看起来很杂。
2. **徽章取舍**：几乎都有包版本（npm 或 PyPI）和 CI。其次是 Node/运行时版本、License、社区（Discord）。下载量只在成熟项目上出现，新包下载数少，显示出来反而减分。
3. **README 长度**：类型或 SDK 包都偏短（Figma 33 行、Obsidian 69 行、Vite 66 行），细节交给文档站。"太长"的项目（Bun、uv）都是把功能目录写进了 README。
4. **快速开始**：最多三步。安装一行；最小代码 3 到 10 行；运行或验证一行。Deno 和 Slint 都是这个形态。
5. **示例**：有截图表格的形式（Slint）和"名称 | 一句话 | 用到的 API"表格的形式（VS Code samples、Tauri plugins），都放在独立的 `examples/` 或单独仓库里，每个示例自带 README 和截图。
6. **脚手架**：生态成熟的都有 `create-*` 或 `init`，或者 GitHub Template 仓库。README 的快速开始因此可以只写一行命令。
7. **双语**：调研的这些仓库都没有在 README 内做语言切换。中文项目常见的做法是 `README.md` 加 `README.zh-CN.md`，顶部放一行 `English | 简体中文` 链接。这个做法来自对中文开源项目的一般了解，本次没有抓取具体仓库验证。
8. **CONTRIBUTING**：小项目（Slint、Raycast、mpv）都很短，核心是 PR 规则、issue 规则和许可声明，构建细节下沉到 docs/。大项目（Vite、uv、Bun）才把开发环境写满。新近的仓库普遍有 **AI 使用政策** 一节（Vite、uv、Tauri）。

### GitHub、npm、TypeDoc 三处的渲染差异

| 写法 | GitHub | npmjs.com | TypeDoc 0.28（我们的文档站首页） |
|---|---|---|---|
| `<p align="center">` / `<div align="center">` | 支持 | 多数可以，但不保证（npm 的 sanitizer 有过滤） | 原样输出 HTML，支持 |
| `<picture>` 加 `prefers-color-scheme` | 支持 | **不支持**：会退回内部的 `<img>`，或者两张都显示（来源：[strip-gh-theme-links](https://cdn.jsdelivr.net/npm/strip-gh-theme-links@5.0.0/README.md)、[Stefan Judis](https://www.stefanjudis.com/blog/how-to-define-dark-light-mode-images-in-github-markdown)） | 浏览器原生支持，但 TypeDoc 的主题切换不是系统偏好，可能对不上 |
| `#gh-dark-mode-only` | 已弃用 | 两张都显示 | 两张都显示 |
| 相对图片路径 `docs/assets/logo.png` | 支持 | 依赖 package.json 的 `repository` 字段把路径解析到 GitHub 仓库（来源：[Node-RED 讨论](https://discourse.nodered.org/t/readme-information-missing-from-https-flows-nodered-org-node-page-for-a-new-node/80065)、[gh-md-urls](https://npmjs.com/package/gh-md-urls)），按默认分支解析 | 会把图片复制进 `media/`，前提是本地有这个文件 |
| GitHub Alerts `> [!NOTE]` | 支持 | 显示成普通引用块，带字面的 `[!NOTE]` | 0.26 以上支持 |
| 徽章（shields.io 的 `<img>`） | 支持 | 支持 | 支持 |

**推论**：logo 用**绝对 URL**，指向 `raw.githubusercontent.com/.../main/...`，并且只用单一一个 `<img>`。我们的渐变 V 深浅色背景都可用，正好省掉 `<picture>`。不要用 Alerts，不要用 `<picture>`。`<p align="center">` 可以放心用；就算在 npm 上失效，也只是退化为左对齐，不影响内容。

---

## 3. README 改写建议

### 3.1 原则

- README 只回答四个问题：**这是什么、长什么样、怎么装、怎么写第一个插件**。其余内容全部链出去，去处有 Getting started、API 参考、examples/、CHANGELOG、CONTRIBUTING。
- 目标 **60 到 90 行 Markdown**，首屏内显示 logo、徽章、tagline 和安装命令。
- 维护者内容（Pages、CI、发布）**全部移出 README**。
- `Versions` 表只保留 "SDK | apiVersion | 最低播放器版本" 三列，每格一个短值。1.1 新增的 API 列表移到 CHANGELOG。
- 修复 `docs/getting-started.md` 里 "Node 22.18" 的过期描述。

### 3.2 章节清单（按顺序）

1. **Hero 区**（居中）
   - logo：`<img src="https://raw.githubusercontent.com/vestrasys/vestra-player-sdk/main/.github/assets/logo.png" width="96" alt="Vestra Player">`
   - `<h1>Vestra Player SDK</h1>`
   - tagline：一行粗体，例如 **Write plugins for Vestra Player in TypeScript — with Slint UI, a test host and a one-command packager.**
   - 徽章行（见 3.3）。
   - 导航行：`Documentation · Getting started · Examples · Changelog · 简体中文`
2. **截图一张**（可选，但强烈建议）：播放器里插件窗口或面板的实际截图，宽 720 左右，用绝对 URL。用户批评的"不 fancy"，最直接的解法就是一张图。
3. **Features**：4 到 6 条短要点，每条一行，可以带 emoji，与 Vite 风格一致：
   - 完整的类型化插件 API（`vestra/1`）：命令、窗口、面板、OSD、播放控制、事件
   - 原生 Slint UI 组件，与播放器同一套主题
   - 内存测试宿主 `@vestra-player/sdk/testing`，不用启动播放器就能单测
   - `vestra-plugin` CLI：check、test、build、pack 生成 `.vxp`
   - macOS 和 Windows 上 API 一致；每个插件运行在独立的 Node 26 进程
4. **Install**：一行 `npm i -D @vestra-player/sdk`，下面一行小字 "Requires Node.js ≥ 26"。删掉现在那段关于 Node 和 TypeScript 的解释，挪到 Getting started。
5. **Quick start**：三个代码块，合计不超过 15 行。
   - `manifest.json`：保持一行 JSON。
   - `main.ts`：保持现在的 5 行。
   - 运行：`npx vestra-plugin test && npx vestra-plugin pack`。将来有 `init` 命令后，把这三块换成一行 `npx vestra-plugin init my-plugin`。
   - 结尾一行链接："→ Full walkthrough: Getting started"。
6. **Examples**：表格，见第 5 节。
7. **CLI**：保留现在的 4 行 `text` 块。它本身就很紧凑，是 Bun 和 yo code 的风格。
8. **Package entry points**：保留现在的 3 行表格。也可以并进第 9 节，作为 "Documentation" 下的一个小表。
9. **Compatibility**：三列小表，SDK | apiVersion | 最低播放器版本，每行一个短值，后面一句 semver 承诺。
10. **Contributing**：一行，链到 CONTRIBUTING.md。
11. **License**：一行 Apache-2.0。

可选：如果在 README 末尾加 "Related"，链到播放器主仓库、插件商店和官方插件仓库，有助于在 npm 上做转化。

### 3.3 徽章（5 个，推荐使用 shields.io 统一风格）

```html
<p align="center">
  <a href="https://www.npmjs.com/package/@vestra-player/sdk"><img src="https://img.shields.io/npm/v/@vestra-player/sdk?logo=npm&color=cb3837" alt="npm version"></a>
  <a href="https://github.com/vestrasys/vestra-player-sdk/actions/workflows/main.yml"><img src="https://img.shields.io/github/actions/workflow/status/vestrasys/vestra-player-sdk/main.yml?branch=main&label=CI&logo=github" alt="CI"></a>
  <a href="https://vestrasys.github.io/vestra-player-sdk/"><img src="https://img.shields.io/badge/docs-TypeDoc-6f42c1" alt="docs"></a>
  <a href="https://www.npmjs.com/package/@vestra-player/sdk"><img src="https://img.shields.io/node/v/@vestra-player/sdk?logo=node.js&logoColor=white" alt="node"></a>
  <a href="https://github.com/vestrasys/vestra-player-sdk/blob/main/LICENSE"><img src="https://img.shields.io/npm/l/@vestra-player/sdk" alt="license"></a>
</p>
```

- **CI 徽章用 `main.yml`（name: Main），不用 `ci.yml`。** `ci.yml` 不在 main 分支上触发（它覆盖的是 push 到非 main 分支和 PR），所以 `?branch=main` 会取不到状态或者显示过时状态。`main.yml` 是 main 分支的真实状态，也包含 Pages 部署。徽章标签用 `label=CI`，对外表达更直观。
- 也可以用 GitHub 原生徽章：`https://github.com/vestrasys/vestra-player-sdk/actions/workflows/main.yml/badge.svg?branch=main`。它的文字是 workflow 的 name（即 "Main"），样式与 shields 不一致，所以推荐用 shields。
- 可选的第 6 个徽章，体现插件 API 版本：`https://img.shields.io/badge/plugin%20API-vestra%2F1-ff2e88`（颜色取 logo 的粉色）。
- **不建议现在加**：下载量（新包数字小，显示出来减分）、stars、Discord（目前没有社区频道；如果以后开 QQ 群或 Discord 再加）。`types` 徽章也不需要，npm 页面本身就显示 TS 图标。
- 包名带 scope，shields 的路径直接写 `@vestra-player/sdk` 即可，不需要转义。

### 3.4 Logo 处理

1. 把 `src/assets/brand/logo.png` 缩到 **256×256**（显示用 96px，覆盖 2x 屏幕），用 `oxipng` 或 `pngquant` 压缩，预计不到 30 KB。放到 SDK 仓库的 `.github/assets/logo.png`，或者 `docs/assets/logo.png`。
2. README 里用**绝对 raw URL** 引用。它在 GitHub、npm、TypeDoc 三处都能显示，而且 logo 不进入 npm 包（`files` 不变，包体不增大）。
3. 只用一个 `<img>`，不用 `<picture>`，原因见第 2 节的渲染差异表。如果以后设计出黑白单色版 logo，GitHub 端可以再加 `<picture>`，`<img>` 回退仍然指向彩色版，npm 端不受影响。
4. 可以把 TypeDoc 的 `favicon` 设为这个 logo（TypeDoc 0.28 支持 `"favicon"` 选项），让文档站也统一品牌。
5. 第 3.2 节第 2 项的截图同样放在 `.github/assets/`，用绝对 URL 引用。

### 3.5 双语：推荐"英文主 README + 完整中文镜像"

- 产品以中文为先，插件作者大概率是中文用户。但是 npm、TypeDoc 文档站、TSDoc 注释、CLI 输出和 manifest 字段名都是英文，GitHub 和 npm 搜索面向的也是全球。npm 页面只能显示 `README.md` 一份。
- 推荐做法：
  - `README.md` 写英文，`README.zh-CN.md` 写中文全文。两份结构和代码块完全一致，只有文字不同。因为目标是 60 到 90 行，同步成本很低。
  - 两份的导航行都放语言切换：`English | [简体中文](https://github.com/vestrasys/vestra-player-sdk/blob/main/README.zh-CN.md)`。**用绝对 URL**，npm 页面上的链接才不会 404。
  - 在 `typedoc.json` 的 `projectDocuments` 里加入 `README.zh-CN.md`，让文档站也能找到中文版。
  - 可选：以后把 `docs/getting-started.md` 也做一份中文版 `docs/getting-started.zh-CN.md`。中文插件作者真正需要的是入门指南，README 只起引导作用。
- 另一种做法：中文写 `README.md`，英文写 `README.en.md`。只有在确定"几乎只有中文开发者、npm 曝光不重要"时才选。代价是 npm 和 TypeDoc 首页变成中文，与英文的 API 参考混在一起。**不推荐。**

### 3.6 README 草稿骨架（英文版，约 75 行）

```markdown
<p align="center">
  <img src="https://raw.githubusercontent.com/vestrasys/vestra-player-sdk/main/.github/assets/logo.png" width="96" alt="Vestra Player logo">
</p>
<h1 align="center">Vestra Player SDK</h1>
<p align="center"><b>Build plugins for Vestra Player in TypeScript — Slint UI, an in-memory test host and a one-command packager.</b></p>
<p align="center">(5 badges)</p>
<p align="center">
  <a href="https://vestrasys.github.io/vestra-player-sdk/">Docs</a> ·
  <a href="https://vestrasys.github.io/vestra-player-sdk/documents/Getting_started.html">Getting started</a> ·
  <a href="https://github.com/vestrasys/vestra-player-sdk/tree/main/examples">Examples</a> ·
  <a href="https://github.com/vestrasys/vestra-player-sdk/blob/main/CHANGELOG.md">Changelog</a> ·
  <a href="https://github.com/vestrasys/vestra-player-sdk/blob/main/README.zh-CN.md">简体中文</a>
</p>

<p align="center"><img src="https://raw.githubusercontent.com/.../.github/assets/screenshot.png" width="720" alt="A plugin window in Vestra Player"></p>

## Features
- (5 bullets)

## Install
npm i -D @vestra-player/sdk   # Node.js ≥ 26

## Quick start
manifest.json (1 line) / main.ts (5 lines) / npx vestra-plugin test && npx vestra-plugin pack
→ [Getting started](...) walks through windows, tests and packaging.

## Examples
(table, see §5)

## CLI
(4-line text block)

## Compatibility
| SDK | Plugin API | Player |
| 1.0 | vestra/1 | ≥ 0.1.0 |
| 1.1 – 1.2 | vestra/1 | ≥ 0.2.0 |
Minor releases add backwards-compatible API only; breaking changes bump `apiVersion` and the major version.

## Contributing · License
[CONTRIBUTING.md](...) · [Apache-2.0](LICENSE)
```

注意两点：

- 所有指向仓库内文件的链接（CHANGELOG、CONTRIBUTING、examples、zh-CN）都要用 `https://github.com/vestrasys/vestra-player-sdk/blob/main/...` 绝对链接，npm 和 TypeDoc 上才能点。现在的 README 已经是这样做的，继续保持。
- `LICENSE` 用相对路径即可。npm 会自动处理，TypeDoc 也能找到这个文件。

---

## 4. CONTRIBUTING 建议

### 4.1 拆成"贡献者"和"维护者"两份

- **`CONTRIBUTING.md`**（面向外部贡献者，目标 50 到 70 行，参照 Slint）：
  1. 欢迎语，说明能从哪些方面帮忙：bug、文档、示例、类型改进。
  2. **提问和报告问题**：bug 用 issue 模板，必须附 SDK 版本、播放器版本、操作系统和最小复现。借用 mpv 的原则："报告你观察到的现象，而不是你的推断"。插件开发的问题去 Discussions。
  3. **API 变更先讨论**：SDK 是 `vestra/1` 稳定 API，必须与播放器保持一致。任何公共 API 的新增或修改都要先开 issue 讨论，因为需要播放器侧配合实现。这一条对我们最关键（Slint 也有同类条款）。
  4. **开发环境**：Node 26，`npm ci`，以及现在那组命令中最常用的 5 条（check、lint、format:check、test、docs）。完整列表链到维护者文档。
  5. **PR 规则**：一个 PR 只做一件事；改了公共 API 要更新 TSDoc（`npm run docs` 把警告当错误）、测试和 CHANGELOG 的 Unreleased 段；CI 里的 `ci-ok` 必须通过；squash 合并。
  6. **Commit 和 PR 标题风格**：一句祈使句或者 `scope: summary`。Conventional Commits 只在打算用工具自动生成 changelog 时才值得引入，现在 CHANGELOG 是手写的，可以不引入。
  7. **Slint 文件是只读副本**：`slint/` 下的文件由播放器同步，不接受直接修改，请到播放器仓库提。
  8. **AI 辅助贡献政策**（可选，Vite、uv、Tauri 都有）：可以用 AI，但提交者必须理解并验证每一行；不接受未经审阅的生成内容。
  9. **行为准则**：链到 `CODE_OF_CONDUCT.md`，用 Contributor Covenant 2.1，目前不存在，需要新建。
  10. **许可**：贡献按 Apache-2.0 授权（inbound = outbound），一句话即可，不需要 CLA。
- **`docs/maintaining.md`**（或 `.github/MAINTAINING.md`）：把现在 CONTRIBUTING 的全部内容原样迁过去，包括构建和 pack 检查、TypeDoc 规则、Pages、CI 拓扑、发布、Trusted Publishing、与播放器同步。在 CONTRIBUTING 末尾放一行 "Maintainers: see docs/maintaining.md"。
  - 注意：`docs/` 下的文件如果不加进 `projectDocuments`，就不会出现在 TypeDoc 站上，这正好符合需要。
- 同时补齐 `.github/`：`ISSUE_TEMPLATE/bug.yml`（包含 SDK 版本、播放器版本、OS、复现步骤）、`ISSUE_TEMPLATE/config.yml`（把提问引到 Discussions）、`PULL_REQUEST_TEMPLATE.md`（3 条勾选：测试、文档、CHANGELOG）。可选 `SECURITY.md`。

### 4.2 中文

CONTRIBUTING 同样建议英文为主。如果目标贡献者以中文为主，可以加一份 `CONTRIBUTING.zh-CN.md`。维护者文档只写一种语言即可。

---

## 5. examples/ 布局与 README 呈现

### 5.1 目录（从播放器 `src/examples/` 迁入，再补一个最小示例）

```
examples/
  README.md                ← 索引表（与主 README 的表一致，或者由脚本生成）
  hello/                   ← 新增：最小示例，命令加 OSD，与 README 的 Quick start 完全一致
    manifest.json  main.ts  tests/main.test.ts  README.md
  panel-menu/              ← ui.mount 面板 + 播放控制
    manifest.json  main.ts  panel.slint  tests/  README.md  screenshot.png
  bookmark/                ← ui.window + 事件 + 播放读写：完整的小插件
  core-ui/                 ← @vestra/ 主题与核心组件、OSD
  markdown/                ← ui.markdown 窗口、manifest 显示字段（中文 name/description）、icon
```

- 每个示例必须包含 `manifest.json`、`main.ts`、`tests/`、`README.md`（10 到 20 行：做什么、用到哪些 capability 和 API、怎么运行）和 `screenshot.png`（`hello` 除外）。这是 VS Code samples 承诺的那套固定结构。
- 示例**不放进 npm 包**（`files` 不变）。在 CI 的 `_test.yml` 里加一步 `vestra-plugin check examples/* && vestra-plugin test examples/*`，确保示例跟随 API 演进、不会腐坏。也可以顺便把 `vestra-plugin pack examples/*` 当作额外的冒烟测试。
- **单一来源**：迁移后需要决定是播放器 `src/examples/` 引用 SDK 仓库的版本，还是两边保持同步并加一个一致性测试（与 Slint 副本的做法相同）。推荐示例只放在 SDK 仓库，播放器如果需要做 GUI 测试，从 SDK 仓库取。
- 示例 id 统一用 `examples.<name>`。`bookmark` 现在是 `community.bookmark`，需要统一。

### 5.2 README 里的呈现

用 Tauri plugins-workspace 加 VS Code samples 的混合形式，一张表。将来示例有截图后，可以在表格上方放一行 2 到 3 张缩略图（Slint 的做法）：

```markdown
## Examples

| Example | Shows | Capabilities |
| --- | --- | --- |
| [hello](…/examples/hello) | A command and an on-screen message — the Quick start, runnable | `commands`, `ui.osd` |
| [panel-menu](…/examples/panel-menu) | A Slint panel mounted in the player, driving playback | `ui.mount`, `playback.control` |
| [bookmark](…/examples/bookmark) | A window that saves and jumps to bookmarks | `ui.window`, `events`, `playback.read` |
| [core-ui](…/examples/core-ui) | The `@vestra/` theme and widgets | `ui.mount`, `ui.osd` |
| [markdown](…/examples/markdown) | A Markdown document window, display fields and icon in the manifest | `ui.window` |

Run one: `cd examples/hello && npm i && npx vestra-plugin test`
```

### 5.3 后续：脚手架（不在本次改写范围，但它决定 Quick start 的最终形态）

- 短期：把 `examples/hello` 做成可以直接复制的模板，或者另建一个 GitHub Template 仓库 `vestra-plugin-template`（Obsidian 和 OBS 的模式），README 里写 "Use this template"。
- 中期：增加 `vestra-plugin init [dir] [--template hello|window|panel]`，从 `examples/` 复制并改写 id，或者发布 `create-vestra-plugin`，支持 `npm create vestra-plugin@latest`（Vite、Tauri、Raycast 的模式）。到那时 README 的 Quick start 可以压缩成：

```sh
npm create vestra-plugin@latest my-plugin
cd my-plugin && npm test && npm run pack
```

---

## 附：待办清单（按优先级）

1. 处理 logo：缩到 256px 并压缩，放到 `.github/assets/logo.png`，在 README 里用绝对 raw URL 引用。
2. 按第 3.2 和 3.6 节重写 README.md（60 到 90 行）。同步写 README.zh-CN.md，并加入 TypeDoc 的 projectDocuments。
3. 截一张播放器内的插件截图，放到 `.github/assets/`。
4. 把现在的 CONTRIBUTING 迁到 `docs/maintaining.md`，新写面向贡献者的 CONTRIBUTING，补 CODE_OF_CONDUCT、issue 模板和 PR 模板。
5. 建 `examples/`：从播放器迁入 4 个示例，新增 `hello`，并在 CI 里检查和测试这些示例。
6. 修正 `docs/getting-started.md` 里的 "Node 22.18"。
7. 可选：TypeDoc 的 `favicon`；将来做 `vestra-plugin init` 或 `create-vestra-plugin`。
