# SDK 文档编写计划

2026-10-22。依据两份调研：[research-readme.md](research-readme.md)（12 个仓库的 README 与 7 份 CONTRIBUTING）和 [research-api.md](research-api.md)（MATLAB、man-pages、MDN、Rust、Stripe、VS Code、Qt、OpenCV、Node.js、Diátaxis、TypeDoc）。

## 现状

- README 73 行纯文字：没有 logo、徽章、截图；插件作者、深入使用者、维护者三类内容混在一页（Pages、CI 这类维护内容也在里面）。
- CONTRIBUTING 其实是维护者手册；外部贡献者要的东西（报问题、PR 规则、行为准则、授权）都没有。
- API 注释覆盖率够（每个导出都有），**缺的是深度和结构**：`types.ts` 103 个导出类型、约 390 个成员；`@throws` 只有 3 处，`@remarks`/`@see`/`@since`/`@category` 为 0；79 个 `@example` 没有一个做过类型检查，很多用了未定义的变量。
- 概念材料其实写过：播放器仓库的中文 `reference.md`、`tutorial.md`、`examples.md` 约 2950 行，但 SDK 文档站链接不到它们。
- 事实漂移：错误码表缺 `versionMismatch`、`host`、`ui-prepare`；重复窗口 id 抛的是没有 `code` 的普通 Error；getting-started 还写 Node 22.18。
- SDK 仓库没有 `examples/`。播放器仓库有 4 个：bookmark、core-ui、markdown、panel-menu。

## 目标

1. 插件作者打开 README，30 秒内知道这是什么、怎么装、怎么跑起第一个插件。
2. 每个 API 都回答：它是什么、什么时候用、怎么用（能编译的例子）、会怎样失败、需要什么权限。
3. 文档里的代码都能编译、能运行，CI 保证不过期。
4. SDK 文档成为插件 API 的唯一权威来源；播放器里的 `reference.md` 改为跳转页。

## 1. README（GitHub、npm、TypeDoc 首页三处同一份）

60–90 行，结构：

```
<居中 logo>
Vestra Player SDK
一句 tagline
[npm 版本] [CI] [docs] [Node ≥26] [License]
Docs · Getting started · Examples · Changelog · 简体中文
<一张截图：示例插件运行在播放器里>
Features（5 条）
Install（一行）
Quick start（≤10 行代码 + 一行运行命令）
Examples（表格：名称 | 演示什么 | 用到的 API）
CLI（vestra-plugin 子命令一张小表）
Compatibility（SDK / 播放器 / Node 对照）
Contributing · License（各一行）
```

- 徽章用 shields.io；CI 徽章指 `main.yml`。下载量、Discord 现在不加。
- Logo：用 `src/assets/brand/logo.png` 缩到 256px，放 SDK 仓库 `.github/assets/logo.png`，README 用 raw 绝对地址（npm 只认绝对地址，也不支持 `<picture>` 和 `> [!NOTE]`）。
- Pages、CI、发布流程这类维护内容全部移出 README；1.1 新增 API 的长列表移进 CHANGELOG。

## 2. CONTRIBUTING

拆成两份：
- `CONTRIBUTING.md`（50–70 行，给外部贡献者）：怎么报问题；公共 API 改动先开 issue 讨论；开发命令；PR 规则；`slint/` 是播放器的只读副本；AI 使用政策；行为准则；Apache-2.0 授权。另加 issue 模板和 PR 模板。
- `docs/maintaining.md`：现有内容整体迁过去（发布、Pages、CI、可信发布）。

## 3. 示例（`examples/`）

播放器仓库的 4 个示例全部迁到 SDK 仓库（播放器仓库删掉，插件仓库以后再同步），再新增两个：

| 示例 | 演示 | 状态 |
| --- | --- | --- |
| `hello` | 和 README 快速开始一字不差：一条菜单命令 + 屏幕提示 | 新写 |
| `bookmark` | 命令、工具窗口、读写播放状态 | 迁移 |
| `panel-menu` | 嵌入面板、菜单 | 迁移 |
| `core-ui` | 核心 UI 组件、挂载、OSD | 迁移 |
| `markdown` | Markdown 显示组件、本地和远程图片 | 迁移 |
| **`blur`（旗舰示例）** | **实时高斯模糊**：菜单开关、参数窗口（强度滑块、步数选择、实时预览、复位）、快捷键、设置页记住参数、屏幕提示；用 `plugin.video.filter()` 的 `gblur` 节点，连续拖动只保留最新值 | 新写 |

- 每个示例自带 README（含截图）和测试；不进 npm 包；CI 对每个示例跑 check/test/pack，保证不过期。
- 示例代码同时是文档的代码来源：文档用 TypeDoc 的 `{@includeCode examples/blur/main.ts#region}` 引入，不复制粘贴。
- 以后再做 `vestra-plugin init <模板>`，快速开始就能压成一行命令（这轮不做）。

## 4. API 文档（文档站）

### 4.1 站点结构（Diátaxis 四类分开写）

| 类别 | 内容 | 篇数 |
| --- | --- | --- |
| 教程 | 从零写第一个插件；加一个工具窗口；加一个设置页；打包发布到商城 | 4 |
| 概念 | 插件与进程模型；权限（capabilities）；命令与菜单；工具窗口、面板与设置页（「窗口到底是什么」）；播放器状态与事件；滤镜链；存储、密钥与数据目录；包与商城；错误与恢复 | 9 |
| 操作指南 | 短篇「怎么做 X」：拖动滑块不卡、记住窗口位置、跨平台路径、调用外部程序、测试插件…… | 约 20 |
| API 参考 | TypeDoc 生成，按 `@category` 分组：界面、播放、滤镜、命令、存储、网络、包管理、测试 | — |

概念篇的素材主要来自播放器仓库 `reference.md` / `tutorial.md` / `examples.md`（约 2950 行）。

### 4.2 每个符号的注释模板

参照 MATLAB（语法 / 说明 / 示例 / 参数 / 提示 / 版本历史 / 另见）、man-pages（ERRORS、NOTES）、Rust（`# Errors`、`# Examples`）：

```
一句话摘要（它做什么）。

@remarks
背景：它是什么、什么时候用、和相近 API 的区别；生命周期；平台差异。

@capability ui.window          ← 自定义标签：需要的权限
@param  … 取值范围、单位、默认值
@returns …
@throws {@link PluginError} `code: "invalid"` — 什么情况下
@throws …（每个错误码一条）
@example 能编译的完整片段（或 {@includeCode} 引示例文件）
@see 相关 API、概念篇
@since 1.1.0
```

调研报告 §3.3 已按这个模板完整重写了 `Windows` / `Windows.create` / `Window` 作为样稿。

### 4.3 工具链

- `typedoc.json`：`@category` 分组；文档和注释可搜索；`typedoc-github-theme`；`typedoc-plugin-mdn-links`（Node/Web 类型链到 MDN）；`projectDocuments` 收录教程、概念、操作指南。
- 例子必须能编译，三层检查：
  1. `examples/` 是真实工程，CI 跑 `tsc` 和测试；
  2. 注释和 Markdown 里的内联代码块抽出来跑 `tsc`；
  3. 示例用 SDK 的测试宿主实际跑一遍。
- 链接：站内链接 TypeDoc 已校验；外链每周用 lychee 扫一次。
- `doc-lint` 脚本：检查公开方法有没有摘要、`@throws`、`@example`、`@category`；检查文档里的错误码和代码里的错误码一致。

### 4.4 工作量

- 约 30 个模块入口类型按完整模板重写（背景、生命周期、示例）。
- 约 180 个方法补错误码和权限标签，其中约 90 个需要能编译的示例。
- 约 60 个数据字段补单位和取值范围。
- 约 108 处补 `@category`，修 6 处事实漂移。
- 工具链 1–2 天。

## 5. 执行顺序

1. **基础**：README（含 logo、徽章、截图）、CONTRIBUTING 拆分、示例迁移、`hello`、`blur`、工具链（主题、分组、例子编译检查、doc-lint）。完成后发 SDK 1.2.x 文档更新。
2. **样板**：先写「界面」这一类（概念篇「工具窗口、面板与设置页」+ `Windows`/`Window`/`Panels`/`Settings` 的参考注释 + 两篇操作指南）。**请用户评审文风和深度**，定稿后再推开。
3. **铺开**：按类别推进其余参考注释和概念篇（播放 → 滤镜 → 命令 → 存储 → 网络 → 包管理 → 测试），每类一个 agent，可并行。
4. **收口**：教程 4 篇；播放器仓库的 `reference.md`、`tutorial.md`、`examples.md` 改为跳转页；修事实漂移（重复窗口 id 要带 `code`、错误码表补全，这几处涉及 SDK 代码，按 API 改动流程走）。

分工照老流程：每块一个 agent，SDK 维护者审 API 事实，协调者审文风、跑 CI、真机截图（README 和示例的截图需要播放器真机运行）。

## 6. 需要用户决定

1. **文档语言**。两份调研意见不同：
   - 调研一：README 英文为主，另附完整的中文版，因为 npm、API 文档、CLI 都是英文；
   - 调研二：文档站和注释以中文为主（产品中文优先，概念材料现在就是中文），README 保留英文简介。
   - 协调者建议：**README 英文 + 中文两份；文档站（教程、概念、指南）中文；API 注释英文**。注释会显示在每个插件作者的编辑器里，英文覆盖的人最多；概念篇是中文素材，翻译成本最高，先用中文。以后再考虑英文版文档站。
2. **样板类别**：先写「界面」，可以吗？
3. **旗舰示例**：高斯模糊（菜单 + 参数窗口 + 实时效果）就够，还是另外再要一个？

## 7. 用户裁决（2026-10-22）

1. **全部用英文**：README（不另出中文版）、文档站（教程、概念、操作指南）、API 注释、示例讲解。
2. 样板由协调者按调研决定：先写「界面」（UI）类。
3. 旗舰示例加第二个，用例要和 blur 不同、有一定复杂度、覆盖视频音频流：定为 **`signal-generator`**——插件自己生成画面和声音（`player.openStream`）：测试图案、显示自身波形的示波器、时间码叠加；声音可选波形、频率、电平；控制窗口实时调整画面和声音；用 `plugin.audio.filter` 演示音频处理。
4. 示例要逐步、必要时逐行讲解（调研结论一致：OpenCV、MATLAB 的示例都是分步讲解，代码从真实示例文件引入）。

执行：基础（维护者：README、CONTRIBUTING、工具链、迁移 4 个示例 + hello）与两个旗舰示例（各一个 agent）并行；之后「界面」样板；再按类别铺开。
