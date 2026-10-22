# `@vestra-player/sdk` API 文档重写调研

日期：2026-10-22。范围：只读调研，不改代码。
对象：SDK 仓库克隆 `scratchpad/sdkrepo`（v1.2.0，TypeDoc 0.28.20，站点 https://vestrasys.github.io/vestra-player-sdk/），以及播放器仓库的长篇参考 `doc/plugins/reference.md`（1658 行）、`tutorial.md`（357 行）、`examples.md`（939 行，34 节）。

---

## 0. 结论先行

1. **问题不只是注释写得短，而是概念材料放错了地方。** 背景知识（更新轮、名称转换、句柄、面板的 global 桥接、全屏侧栏、错误码的真实含义）都写在播放器仓库的中文 `reference.md` / `tutorial.md` / `examples.md` 里，SDK 站点一个也链接不到。SDK 站点上只有英文的 TSDoc 加一篇 getting-started，而 TSDoc 本身是"规格说明"的文风：短、密、默认读者已经懂概念。
2. 好文档的共同做法（MATLAB、man、Rust、MDN、Stripe、Qt、VS Code、OpenCV、Django、Node）是：**概念页和参考页分开写，而且互相链接**。每个符号的参考条目都按固定的小节顺序写（摘要、说明、参数、返回值、错误、示例、另见、版本），示例都是完整可运行的，并且由机器检查。
3. 建议的做法：
   - 站点按 Diátaxis 分成四类：教程、概念、操作指南、API 参考。
   - 播放器仓库里的中文长文档迁到 SDK 仓库，成为**唯一的权威来源**；`reference.md` 降级为跳转页。
   - TSDoc 按统一模板补齐 `@remarks`、`@throws`（我们的错误码）、自定义 `@capability`、`@example`、`@see`、`@since`。
   - 示例写成 `examples/` 下真实的 `.ts` 文件，用 `{@includeCode}` 引入文档，CI 里用 `tsc` 检查，并用 `startPlugin` 测试宿主实际运行一遍（相当于 doctest）。
   - 语言：**中文为主**，设 `lang: "zh"`；README 和 npm 入口保留英文简介。
4. 工作量：需要完整按模板重写约 30 个"模块入口"类型和约 140 个方法；补单位和默认值的属性约 60 个；另写概念页约 9 篇、操作指南约 20 篇（大部分可以从 `examples.md` 改写）。

---

## 1. 现状评估

### 1.1 量化

| 指标 | 数值 | 说明 |
| --- | --- | --- |
| `types.ts` 导出类型 | 103 个 | 大约 390 个成员，其中约 137 个是方法 |
| 单行 `/** … */` 注释 | 434 条（types.ts），172 条（testing.ts） | 多数方法只有一句话 |
| `@example` | 79 处（types.ts 44，testing.ts 28） | 示例**不做类型检查**：`tsconfig.check.json` 只覆盖 `src/`、`test/`，示例里常出现未定义的变量（`store`、`load`、`stopPolling`、`askUser`、`chain`、`settings`） |
| `@throws` / `@returns` / `@param` | 3 / 7 / 23 | 错误大多写在正文里，比如 "`invalid` for …" |
| `@remarks`、`@see`、`@since`、`@defaultValue`、`@category` | **全部为 0** | 没有版本信息，也没有分组，侧栏按源码顺序平铺 103 个类型 |
| 站点里的手写页面 | 1 篇（`docs/getting-started.md`，239 行） | `projectDocuments` 里只有这一篇 |
| TypeDoc 校验 | `notDocumented`、`invalidLink` 开启，警告视为错误 | 底子不错：每个导出都有注释，链接也不会断 |

结论：**覆盖率没有问题，问题在深度和结构。** 每个符号都有注释，但几乎都只有"是什么"，没有"为什么、在整体里处于什么位置、怎么用、会怎么失败"。

### 1.2 五个典型薄弱点（附原文）

#### ① `Windows.create` / `Window`：没有回答"工具窗口到底是什么"

```ts
/** `plugin.windows`: tool windows. Needs `ui.window`. */
export interface Windows {
  /**
   * A tool window from a `.slint` file in the package. Nothing opens until `show()`.
   ...
```

```ts
/**
 * A tool window, from {@link Windows.create}. Disposing it closes the
 * window.
 * @example
 * const info = plugin.windows.create("ui/info.slint");
 * info.on("refresh", () => load());
 * info.onClose(() => stopPolling());
```

缺了什么：

- **概念**：注释没说它是一个独立的系统级顶层窗口。看播放器源码 `src/host/src/ui.rs::open`，它是 Slint 实例自己的原生窗口：有自己的标题栏，非模态，不依附主窗口；尺寸和标题来自 Slint 的 `Window` 组件；显示之前播放器会套上主题色和系统字体。注释也没说它和面板、设置页、对话框有什么区别，什么时候该选哪个。
- **生命周期**：`create → on → show → hide/close → 用户关闭 → dispose` 这条线没有完整讲过，"关闭后再 `show()` 是一个全新实例"散落在三个成员的注释里。
- **标准关闭键**：用户按 ⌘W，或 Windows 上的 Ctrl+W / Ctrl+F4，等同于点标题栏关闭，会触发 `onClose`。这一点只写在 `reference.md` §6.2。
- **错误**：`show()` 可能以哪些 code 拒绝，完全没写。按宿主代码（`main.rs` 的 `"ui.open"` 分支）推断，至少有：`unavailable`（无界面模式）、`capability`、文件越界或不存在（`plugins::package_file` 的错误）、Slint 编译失败。后者由 `From<anyhow::Error>` 映射成 **`host`**，而这个 code 不在 `PluginError` 的错误表里。
- 重复 id 抛出的是**不带 `code` 的普通 `Error`**（`ui.ts`：`throw new Error(\`window ${id} already exists…\`)`），与 `PluginError` 承诺的"每个失败都有稳定的 code"相矛盾。
- **示例**：只有 TS 一半，没有对应的 `.slint` 文件；`load`、`stopPolling` 未定义，示例复制下来跑不起来。

#### ② `Player` 的控制方法：一句话，没有范围、错误和条件语义

```ts
/** Pause or resume. */
setPaused(paused: boolean, options?: Conditional): Promise<void>;
/** Jump to `seconds`. */
seek(seconds: number, options?: Conditional): Promise<void>;
/** Set the playback speed. */
setRate(rate: number, options?: Conditional): Promise<void>;
```

`setRate` 接受的范围、`seek` 超出时长时的行为、不可跳转的来源会不会 `unsupported`、没有媒体时是不是 `unavailable`，都没写。`stale` 的条件提交语义只在 `Player` 接口顶部提了一句。长篇参考 §5.2 同样只写了"跳转""倍速"。这类方法正是 MATLAB 的 Input Arguments 和 man 的 ERRORS 最擅长覆盖的部分。

#### ③ `Settings.page` / `Panels`："one component and one global" 的桥接概念从来没解释

```ts
/**
 * Add a page (the file exports one component and one global, like a
 * panel). ...
 * @example
 * page.bind(() => ({ resume: store.value.resume }));
 * page.on("resumeChanged", (on: boolean) => store.save({ ...store.value, resume: on }));
```

- 为什么窗口是"一个组件"，而面板和设置页是"组件 + global"？global 在这里起什么作用，属性该写在哪？这些都是 Slint 新手最大的障碍。tutorial.md 有"界面和插件之间只有两个方向"一节，但 SDK 站点上看不到。
- 示例里的 `store` 是未定义变量，暗中依赖 `plugin.storage.json(...)`。示例没有给出 `.slint` 文件，也没有处理 `page.ready`（文件编译失败时会以 `ui-prepare` 拒绝）。

#### ④ `Packages`：流程类 API 缺少流程说明，字段值没有枚举

```ts
/** Validate a package directory (`plugins.manage`). */
validate(directory: string): Promise<PackageCandidate>;
/** Restore a plugin's previous version (`plugins.manage`). */
rollback(id: string): Promise<PackageChange>;
...
/** The checks the player ran. */
readonly checks: string[];
/** The trial's status as the player reports it. */
readonly status: string;
```

- 文档没讲"校验 → 试载（trial）或安装 → `change.done` 结果"这条状态机，也没说 validate 具体检查什么（manifest、`minPlayerVersion`，会出 `versionMismatch`）。
- `checks`、`status` 是开放的 `string`，可能取哪些值没有列出。
- 一个"插件商店"插件的作者必须去播放器仓库读 `store-design.md` 才能写对。

#### ⑤ 错误模型和事实在多处漂移

- `PluginError` 的错误表没有 `versionMismatch`，但 `reference.md` §13 有，宿主 `main.rs` 也确实会返回它。宿主还会返回 `host`（Slint 编译失败等）、`ui-prepare`、`ui-value`，SDK 的表里都没有。
- `docs/getting-started.md` 写的是 "Node.js process (**Node 22.18** or newer)"，而 `package.json` 是 `"node": ">=26"`，`reference.md` 也写"内置 Node 26"。示例 `package.json` 里还是 `"@vestra-player/sdk": "^1.0.1"`，当前版本是 1.2.0。
- 同一条规格写在两处（TSDoc 英文，`reference.md` 中文），`reference.md` 开头自己也承认"本文与它不一致时以代码为准"。这是**结构性的漂移来源**。

> 已经写得好的地方，可以当作模板的种子：`Disposable`（列出了每种句柄释放时做什么）、`PluginError`（错误码表）、`Scope`（带子作用域的完整示例）、`CommandOptions.bindings`（语法、平台差异、冲突规则齐全）、`Ui.markdown`（限制、缓存、版本要求齐全）。问题在于这些是例外，不是规范。

---

## 2. 各家优秀文档的做法

### 2.1 MATLAB 函数参考

来源：https://www.mathworks.com/help/matlab/ref/double.interp1.html

- 固定的小节顺序：**Syntax → Description → Examples → Input Arguments → Output Arguments → More About / Tips / Algorithms → References → Extended Capabilities → Version History → See Also**。
- **Syntax 和 Description 一一对应**：每种调用形式（`vq = interp1(x,v,xq)`、`vq = interp1(v,xq)`、`pp = interp1(x,v,method,"pp")`）各配一段描述，读者按"我想这样调用"来查。
- **示例写成任务标题**，比如 "Interpolation of Coarsely Sampled Sine Function"、"Designate Constant Value for All Queries Outside the Domain of x"。每个示例自成一体，可以直接运行。
- **Input Arguments 每项都有固定字段**：名称、形状或类别、说明、约束、数据类型、默认值、示例值，可以折叠。
- **Extended Capabilities** 专门列出平台或环境差异（例如 GPU 上不支持 `"pchip"`）。对应到我们，就是 macOS 和 Windows 的差异，以及无界面模式、软件渲染下的差异。
- **Version History** 按版本列出"Introduced in R20xx"和后续的行为变化。

对我们的启发：

- 错误和平台差异放在独立的小节，不要埋在正文段落里。
- 示例要用"任务"做标题。
- 参数写清范围、单位和默认值。

### 2.2 Linux man-pages 约定

来源：https://man7.org/linux/man-pages/man7/man-pages.7.html

- 小节顺序：NAME、LIBRARY、SYNOPSIS、DESCRIPTION、RETURN VALUE、**ERRORS**、ENVIRONMENT、FILES、ATTRIBUTES、VERSIONS、STANDARDS、HISTORY、NOTES、**CAVEATS**、BUGS、**EXAMPLES**、SEE ALSO。
- **ERRORS 的规范**：列出 errno 的每个可能值及其原因。同一个错误码对应不同条件时，分成多个条目（"separate list entries (with duplicate error names) for each of the conditions"），并按字母排序。这正好对应我们按 `code` 分支的错误模型。
- **EXAMPLES 的规范**：示例程序要短、完整、能无警告编译、带错误检查。
- **CAVEATS / BUGS**：常见误用和已知限制单独成节。对应到我们，就是"回调必须在第一次 `show()` 前绑定"这类陷阱。
- HISTORY / VERSIONS：记录何时出现、何时发生显著变化。

### 2.3 Rust std / rustdoc

来源：https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html ，https://doc.rust-lang.org/std/fs/struct.File.html

- 推荐的模板：一句话说明它是什么；更详细的解释；至少一个可以复制试用的示例；必要时再写进阶说明。
- **摘要行**（第一个空行之前的内容）会被复用到搜索结果和模块概览里，所以必须独立成句。
- 惯用的标题小节：`# Errors`、`# Panics`、`# Safety`、`# Examples`。`File::open` 的 Errors 写的是 "This function will return an error if `path` does not already exist. Other errors may also be returned according to `OpenOptions::open`."
- **类型级文档先讲概念**。`File` 页面在方法列表之前，先讲"离开作用域自动关闭、关闭时的错误会被忽略、需要错误就用 `sync_all`"，以及"不缓冲，请包一层 `BufReader`"和 Windows 的特殊行为。这正是我们 `Window` 缺的那段。
- **示例就是测试**：rustdoc 会编译并运行文档里的代码块，示例因此不会腐烂。
- crate 和模块级文档（`//!`）先用不太技术化的语言讲清"这个模块在生态里的位置"。

### 2.4 MDN Web API

来源：https://developer.mozilla.org/en-US/docs/Web/API/Window/fetch ，https://developer.mozilla.org/en-US/docs/Web/API/Window

- 方法页的小节：Syntax → Parameters → Return value → **Exceptions** → Examples → Specifications → Browser compatibility → See also。
- **Exceptions 的格式是"异常类型 + 触发条件列表"**，例如 `TypeError` 下逐条列出 "The requested URL is invalid"、"…includes credentials" 等。可以直接照搬成"错误码 + 条件"。
- 接口页（`Window`）开头先用两句话定义概念："The `Window` interface represents a window containing a DOM document…"。然后是继承关系，以及按类别分组的属性、方法、事件清单，每项一行摘要，带 Read only / Deprecated / Experimental 徽标。
- **参考页和概念指南互相链接**：`fetch()` 页链接到长篇指南 "Using Fetch"；每个 API 家族还有一个 "X API" 概览页，讲概念和用法、列出接口。示例同时有在线可运行版和 GitHub 源码。

### 2.5 Stripe API 参考

来源：https://docs.stripe.com/api/refunds/create

- 双栏布局：左边是说明和参数，右边是请求代码和**完整的示例响应对象**。
- 开头先讲业务语义，而不是语法："You can optionally refund only part of a charge … Once entirely refunded, a charge can't be refunded again. This method will raise an error when…"。
- **前置条件**（Prerequisites）：明确告诉读者运行这段代码之前要先创建什么对象。
- 参数格式统一为"名称（类型，required/optional）+ 说明 + 可选的枚举值"，嵌套参数可以展开。
- Returns 一节同时写成功返回什么、何时报错，并链接到统一的错误页。

对我们的启发：

- 方法注释的第一段先写"这个操作在业务上意味着什么，以及何时失败"。
- 示例写上前置条件，例如"需要 manifest 里声明 `ui.window`，以及包内有 `ui/info.slint`"。

### 2.6 VS Code Extension API

来源：https://code.visualstudio.com/api/extension-capabilities/overview ，https://code.visualstudio.com/api/extension-guides/webview ，`vscode.d.ts`（https://github.com/microsoft/vscode/blob/main/src/vscode-dts/vscode.d.ts）

- **关键事实：`vscode.d.ts` 本身也很简短。** 例如 `createWebviewPanel` 的注释只有 "Create and show a new webview panel." 加上 `@param` 和 `@returns`。它的深度全部在**概念指南**里：Webview 指南先用一句话定义（"Think of a webview as an `iframe` within VS Code that your extension controls…"），再讲 "Should I use a webview?"，然后依次是生命周期、消息传递、安全、持久化、可访问性。指南链接到 `vscode-extension-samples` 里可以运行的样例。
- 站点结构：Get Started（教程）→ Extension Capabilities（能做什么，概念）→ Extension Guides（每种能力怎么做）→ UX Guidelines → References（API、Contribution Points、Activation Events）。
- 对我们最有用的一点：我们和 VS Code 是同一类"宿主 + 插件 API"。**参考可以写得紧凑，前提是有概念指南并且互相链接。** 我们现在的情况是概念指南存在，但不在站点上，也没有链接。

### 2.7 Qt

来源：https://doc.qt.io/qt-6/qwidget.html

- 类页面的结构：头部表格（Header、Module、Inherits、Inherited By）→ 属性和函数清单 → **很长的 Detailed Description**，下面分概念小节（Top-Level and Child Widgets、Composite Widgets、Size Hints and Size Policies、Events…）→ 成员详述。
- 概念就写在类页面里。例如 "A widget without a parent widget is always an independent window (top-level widget)… Non-window widgets are child widgets, displayed within their parent widgets."，这正是我们 "what exactly is a window" 想要的那种回答。
- 每个成员都带 "See also" 和 "This function was introduced in Qt 6.x"。

### 2.8 OpenCV

来源：教程源码 https://github.com/opencv/opencv/blob/4.x/doc/tutorials/imgproc/imgtrans/canny_detector/canny_detector.markdown ，Python 教程 https://github.com/opencv/opencv/blob/4.x/doc/py_tutorials/py_imgproc/py_canny/py_canny.markdown ，参考注释 https://github.com/opencv/opencv/blob/4.x/modules/imgproc/include/opencv2/imgproc.hpp 。docs.opencv.org 前面有 Cloudflare 挡着，抓取会返回 403，所以这里引用的是生成站点用的源码。

- 教程的固定结构：**Goal → Theory → Code → Explanation → Result**，并用 `@prev_tutorial` / `@next_tutorial` 串成学习路径。
- **教程里的代码不手写**：用 `@include samples/cpp/tutorial_code/ImgTrans/CannyDetector_Demo.cpp` 引入整个文件，用 `@snippet … variables` 引入文件中的命名片段，逐段讲解；`@add_toggle_cpp/java/python` 提供多语言切换。样例是真实的、会被编译的程序，所以文档里的代码不会和 API 脱节。
- 参考条目（`Canny`）：`@brief` 一句话，然后是算法说明加外链，逐个 `@param`；类头部还有 `@example samples/cpp/edge.cpp … Check the corresponding tutorial`，把参考链接回教程。
- **TypeDoc 0.28 的 `{@includeCode file.ts#region}` 就是这套机制的等价物**，见 §4.3。

### 2.9 Django 和 Diátaxis

来源：https://docs.djangoproject.com/en/stable/ ，https://diataxis.fr/

- Django 的四类文档：
  - **Tutorials**："take you by the hand through a series of steps…"；
  - **Topic guides**："discuss key topics and concepts at a fairly high level and provide useful background information and explanation"；
  - **Reference guides**："technical reference for APIs… assume that you have a basic understanding of key concepts"；
  - **How-to guides**："recipes… guide you through the steps involved in addressing key problems"。
- Diátaxis 的两条轴是"行动 / 认知"和"学习 / 应用"。对参考文档的要求是：**描述**而非指导、**简朴**、**结构一致**、**结构镜像产品本身**。
- 对我们的意义：用户的批评（"没有背景""不知道窗口是什么"）属于 **Explanation** 层缺失，不能全靠往参考注释里塞长文来解决。参考里放"必要背景（几句话）+ 链接到概念页"，完整的解释放在概念页。

### 2.10 Node.js API 文档

来源：https://nodejs.org/api/fs.html

- 每个 API 都标 **Stability**（"Stability: 2 - Stable"），并有 **"Added in: vX" 加版本变化表**，例如 "v21.1.0 The `flush` option is now supported."
- 参数格式统一为 "`` `position` <integer> | <null> … **Default:** `null` ``"，返回值写成 "Returns: <Promise> Fulfills upon success with…"。
- 示例同时提供 ESM 和 CJS 两种写法。示例里包含 `try/finally` 释放资源，展示的是正确用法，而不只是最小用法。

### 2.11 TypeDoc / TSDoc 能提供什么

来源：https://typedoc.org/documents/Tags.html ，https://typedoc.org/documents/Options.Output.html ，以及本地 `node_modules/typedoc`（0.28.20）源码核对。

| 需求 | TypeDoc 0.28 支持 | 备注 |
| --- | --- | --- |
| 摘要和详细说明 | 第一段即摘要；`@remarks`；`@summary`（用于列表摘要） | |
| 参数和返回值 | `@param`、`@returns`、`@typeParam`、`@defaultValue` | 内联对象参数可以用 `@param options.id -` 的写法 |
| 错误 | `@throws` | 每个 code 写一条 `@throws`，相当于 man 的 ERRORS |
| 示例 | `@example 标题`（标题会显示出来） | 可以有多个 |
| 引入外部代码 | **`{@includeCode ../examples/windows.ts#basic}`**（按 `#region` 名截取，多个 region 用逗号分隔），`{@includeCode file:10-20}`（按行），`{@include doc.md}` | 已在本地源码核对：路径相对于注释所在文件；按 region 截取支持 `ts`、`css`、`md`、`c`、`cs` 等扩展名，**不支持 `.slint`**，`.slint` 只能按行号或整文件引入 |
| 交叉链接 | `{@link X}`、`{@linkcode}`、`@see` | 已开启 `validation.invalidLink` |
| 版本 | `@since` | 已内置 |
| 分组 | `@category` / `@categoryDescription`、`@group` / `@groupDescription`，`navigation.includeCategories` | 解决 103 个类型平铺的问题 |
| 自定义标签 | `blockTags` 选项加上 `tsdoc.json` 声明 | 用于 `@capability`、`@player`（需要的播放器版本） |
| 手写页面 | `projectDocuments`，frontmatter 支持 `group`、`category`、`children` | 教程、概念、操作指南都可以放进同一个站点 |
| 本地化 | `lang: "zh"`，**内置中文界面翻译** | 本地已核对：`dist/browser-locales/zh.js` 存在 |
| 搜索 | `searchInComments`、`searchInDocuments` | 开启后中文正文也能搜到 |
| 外部类型链接 | `typedoc-plugin-mdn-links` 5.1.1（peer `0.27.x \|\| 0.28.x`） | `AbortSignal`、`Error`、`Promise` 自动链接到 MDN |
| 主题 | 默认主题；`typedoc-github-theme` 0.4.0（`~0.28.0`）；`typedoc-material-theme` 1.4.1（`^0.28.x`） | 另一条路线：`typedoc-plugin-markdown` 4.14 加 `typedoc-vitepress-theme` 1.1.4，交给 VitePress 生成（自带 i18n 和更好的手写页体验） |
| 覆盖率 | `typedoc-plugin-coverage` 4.0.3 | 生成文档覆盖率徽章 |

---

## 3. 我们的文档标准

### 3.1 站点结构（Diátaxis）

```
vestra-player-sdk 文档站
├── 入门（Tutorial）
│   ├── 1. 第一个插件：命令 + 屏幕提示（10 分钟）
│   ├── 2. 加一个工具窗口（Slint ⇄ TS 双向）
│   ├── 3. 测试：用 TestHost 不启动播放器
│   └── 4. 检查、运行、打包、发布到商城
├── 概念（Explanation / Topic guides）
│   ├── 插件是怎么运行的：进程、activate、能力、可信模型、平台
│   ├── 句柄与作用域：Disposable、Scope、子作用域、停用时的清理
│   ├── 更新轮与绑定：bind / set / Bindable、何时重新求值、合并发送
│   ├── 界面总览：工具窗口 / 面板（底栏、侧栏、背景层）/ 设置页 / 对话框 / OSD —— 怎么选
│   ├── Slint 桥接：属性与回调、in / in-out、名称转换、component + global、主题与控件库
│   ├── 命令、菜单与快捷键：路径、order / group、绑定语法、冲突优先级、用户改绑
│   ├── 播放器状态：PlayerState、sourceId、条件提交（ifCurrent / stale）、事件与 events 能力
│   ├── 媒体处理：滤镜链、显示层、渲染选项、生成流
│   ├── 错误模型：code 一览、onError、未 await 的 promise、超时
│   └── 设置与数据：storage、secrets、dataDirectory、settings 页面的分工
├── 操作指南（How-to，大部分从 examples.md 的 34 节改写）
│   ├── 按视频尺寸调整窗口 / 置顶
│   ├── 侧栏播放列表：列表模型、选中、自动推进、右键菜单
│   ├── 在「选项」窗口里加一页设置
│   ├── 保存 API key 到系统密钥库
│   ├── 解析网络地址（Resolver）并等待就绪
│   ├── 截图与导出，显示进度、支持取消
│   ├── 校验、试载、安装插件包
│   └── …
├── API 参考（TypeDoc，按 @category 分组）
│   ├── 核心：Plugin、Scope、Disposable、PluginError
│   ├── 命令与快捷键：Commands、Command、CommandOptions、Bindings
│   ├── 界面：Windows、Window、View、Panels、Panel、Settings、Dialogs、Osd、Ui、Drops、Appearance、MainWindow
│   ├── 播放：Player、Media、Scrub、Stream、Timeline、Host
│   ├── 处理：VideoFilter、AudioFilter、VideoView、RenderOptions、Subtitles、MediaCapture
│   ├── 系统：Storage、Secrets、System、Clipboard、Power、Tools、App
│   ├── 插件管理：Packages、Resolvers
│   └── 测试：@vestra-player/sdk/testing
└── 附录：错误码总表、能力（capability）总表、平台差异表、版本历史（CHANGELOG）
```

几条规则：

- **单一权威来源**：概念和操作指南的正文迁到 SDK 仓库的 `docs/` 下。播放器仓库的 `doc/plugins/reference.md` 只保留实现侧的笔记和"变更说明"，对外的规格一律链接到 SDK 站点。
- **双向链接**：
  - 每个参考条目至少有一个 `@see`，指向对应的概念页；
  - 每篇概念页列出"本页涉及的 API"，用 `{@link}`；
  - 每篇操作指南的代码来自 `examples/`，与参考里的示例共用同一份文件。
- 错误码、能力、平台差异各自有**一张总表**，参考条目只写"本方法会出现哪些 code、在什么条件下"，并链接到总表。

### 3.2 每个符号的注释模板

适用范围：模块入口（`Windows`、`Player`…）和所有方法。纯数据字段只需要摘要，以及单位、范围、默认值。

````ts
/**
 * 〔摘要：一句话，动词开头，说清做什么。会被列表和搜索复用，必须独立成句。〕
 *
 * @remarks
 * 〔背景：2–6 句。它在整体里处于什么位置；生命周期或状态变化；和相近 API 的区别（何时改用 X）；
 *   持续期间的约束（例如"回调必须在第一次 show() 前绑定"）。长背景只写结论，细节链接到概念页。〕
 *
 * **平台**：〔macOS / Windows / 无界面模式 / 软件渲染下的差异。没有差异时省略。〕
 *
 * @capability 〔manifest 能力，例如 ui.window。不需要能力时写"无"。〕
 * @player 〔需要的最低播放器版本，只在高于 vestra/1 基线时写〕
 *
 * @param name - 〔含义；单位；范围；默认值；格式。是否可选由类型表达，这里不重复〕
 * @param options.id - 〔内联对象参数逐个字段写〕
 * @returns 〔resolve 的时机和值。"resolve 时什么已经生效"必须写清〕
 *
 * @throws `invalid` 〔同步抛出 / 异步拒绝，二者分清〕〔条件〕
 * @throws `unavailable` 〔条件〕
 *   〔每个 code 一条；同一 code 有不同条件时分成多条（man-pages 规则）；按"调用方最可能遇到"排序〕
 *
 * @example 〔任务式标题，例如"在菜单里打开信息窗口"〕
 * {@includeCode ../../examples/snippets/windows.ts#info-window}
 *
 * @see {@link 相关符号}、[概念：界面总览](../documents/界面总览.html)
 * @since 1.0.0
 * @category 界面
 */
````

写作规则：

- **摘要**：不超过一行，不以"This"或"一个"开头，不复述类型名。
- **`@remarks`**：必须回答"什么时候用它，什么时候不用"。
- **`@throws`**：只用 `PluginError` 错误码总表里的 code；要写清是同步 `throw` 还是 Promise 拒绝。表外的 code（`host`、`ui-prepare`、`versionMismatch`）先补进总表。
- **`@example`**：
  - 必须能编译：用 `{@includeCode}` 引入 `examples/`，或者内联代码块在 CI 里抽出来用 `tsc` 检查；
  - 必须自成一体，不出现未定义的变量；
  - 涉及界面时同时给出 `.slint`，用 `{@includeCode ../../examples/ui/info.slint:1-12}` 按行号引入；
  - 第一个示例写最常见的用法，第二个示例展示一个陷阱或进阶用法。
- **方法与模块的分工**：模块入口（`Windows`、`Player`）写概念段和整体示例；方法写各自的参数、错误和小示例，不重复概念段。
- **术语表**（中文文档）：
  - 句柄（handle）、作用域（scope）、能力（capability）、更新轮（update pass）、绑定（binding）；
  - 工具窗口（tool window）、面板（panel）、桥接 global（bridge global）、条件提交（conditional）、来源（source）。
  - 代码标识符和错误码保持英文原样。

### 3.3 完整示例：用模板重写 `Windows` / `Windows.create` / `Window`

以下内容的事实依据：SDK `src/plugin/ui.ts` 的 `WindowHandle`、`types.ts`、`reference.md` §6.2 和 §6.4、播放器 `src/host/src/ui.rs::open` 与 `main.rs` 的 `"ui.open"` 分支。标【待核实】的条目需要实现方确认，或者先修代码。

````ts
/**
 * 工具窗口：插件用 `.slint` 文件定义、由播放器打开的独立窗口。
 *
 * @remarks
 * **什么是工具窗口。** 它是一个独立的顶层系统窗口：有自己的标题栏，可以移动、调整大小，
 * 非模态（打开时主窗口照常可以操作、照常播放），不嵌在主窗口里。窗口的内容、初始尺寸和标题
 * 都由 `.slint` 文件里那个 `inherits Window` 的组件决定（`title`、`preferred-width` 等）。
 * 播放器在窗口显示之前套上当前配色（{@link Appearance}）和系统字体，所以插件不需要处理主题。
 *
 * **怎么选界面形式。**
 * - 需要"打开 → 操作 → 关掉"的独立工具（画面调节、信息面板、编辑器），用工具窗口；
 * - 需要常驻在播放器里、随主窗口布局变化的内容（播放列表、进度条、封面），用
 *   {@link Panels.mount | 面板}；
 * - 偏好设置用 {@link Settings.page | 设置页}；
 * - 只需要问一句"确定吗"，用 {@link Dialogs.confirm}。
 * 对比见 [概念：界面总览](../documents/界面总览.html)。
 *
 * **生命周期。** `create()` 只在本地建立句柄，不打开任何东西。之后的流程：
 * 1. 用 {@link View.on} 绑定回调，用 {@link View.bind} 声明属性；
 * 2. 第一次 {@link Window.show} 用当前绑定的属性作为初始值打开窗口，不会闪一下默认值；
 * 3. {@link Window.hide} 隐藏窗口，但保留状态；
 * 4. {@link Window.close}、用户点标题栏关闭、或按标准关闭键（macOS ⌘W；Windows Ctrl+W 或 Ctrl+F4）
 *    都会销毁窗口实例。之后再 `show()` 会用最新属性打开一个全新实例。只有用户关闭时才会调用
 *    {@link Window.onClose}。
 *
 * 句柄属于创建它的 {@link Scope}。作用域释放或插件停用时窗口自动关闭，插件不需要自己清理。
 *
 * **平台**：macOS 与 Windows 行为相同。无界面模式（headless）下 `show()` 以 `unavailable` 失败。
 *
 * @capability ui.window
 * @see [概念：Slint 桥接](../documents/Slint桥接.html)、{@link Window}、{@link View}
 * @since 1.0.0
 * @category 界面
 */
export interface Windows {
  /**
   * 为包内的一个 `.slint` 文件创建工具窗口句柄；在调用 {@link Window.show} 之前不会打开窗口。
   *
   * @remarks
   * 文件必须**恰好导出一个组件**，通常写成 `export component X inherits Window`。不带
   * `export` 的辅助组件数量不限。这一点和面板、设置页不同：面板和设置页要求导出
   * "一个 component 加一个 global"。
   *
   * 窗口只转发**第一次 `show()` 时已经绑定过名字的回调**。这是因为播放器在打开窗口时一次性
   * 登记回调名。所以要在 `show()` 之前用 {@link View.on} 绑定全部回调。之后再绑定新的名字
   * 会同步抛错；同一个名字可以重新绑定。属性没有这个限制。
   *
   * 同一个文件要开多个窗口时（例如每个文件一个信息窗口），用 `options.id` 区分。
   *
   * @param file - 相对于插件包目录的路径，例如 `"ui/picture.slint"`。不能越出包目录。
   * @param options.id - 窗口 id，在本插件内唯一。默认取不带扩展名的文件名
   *   （`ui/picture.slint` → `picture`）。窗口 dispose 之后这个 id 可以再用。
   * @returns 属于当前作用域的 {@link Window} 句柄。
   *
   * @throws （同步）本插件已有同 id 的窗口。
   *   【待核实/待修：目前抛出的是不带 `code` 的普通 Error，应改为 `invalid`】
   *
   * @example 在菜单里打开一个信息窗口
   * {@includeCode ../../examples/snippets/windows.ts#info-window}
   *
   * 对应的 `ui/info.slint`：
   * {@includeCode ../../examples/snippets/ui/info.slint}
   *
   * @example 同一个文件开多个窗口
   * ```ts
   * const left = plugin.windows.create("ui/compare.slint", { id: "compare-left" });
   * const right = plugin.windows.create("ui/compare.slint", { id: "compare-right" });
   * ```
   *
   * @see {@link Panels.mount}、{@link Settings.page}
   * @since 1.0.0
   */
  create(file: string, options?: { id?: string }): Window;
}

/**
 * 一个工具窗口的句柄，由 {@link Windows.create} 得到。
 *
 * @remarks
 * 属性和回调的规则（`bind` 与 `set` 的区别、名称转换、写入失败重试）继承自 {@link View}。
 * 窗口特有的规则只有两条：
 * - 回调要在第一次 `show()` 前绑定；
 * - 用户可以自己关闭窗口。
 *
 * 用户关闭之后，`opened` 变为 `false`，句柄依然可用：再次 `show()` 会打开一个新实例。
 * 窗口里正在进行的工作（轮询、计时器）应该在 {@link Window.onClose} 里停掉。
 *
 * `dispose()` 等于 `close()` 再加上释放句柄：之后 `show()` 什么也不做，这个 id 可以重新 `create`。
 *
 * **测试**：`host.window(id).props` 读当前属性，`host.window(id).userClose()` 模拟用户关闭。
 * 参见 {@link "@vestra-player/sdk/testing"!TestHost}。
 *
 * @capability ui.window
 * @since 1.0.0
 * @category 界面
 */
export interface Window extends View, Disposable {
  /**
   * 打开窗口（第一次调用，或在 `close()` / 用户关闭之后），或者把隐藏的窗口重新显示出来。
   *
   * @remarks
   * 打开时用当前的 `bind` 值和最近一次 `set` 的值作为初始属性。
   * 同一窗口的 `show`、`hide`、`close` 严格按调用顺序执行；同时发起的多次 `show()` 共用一次打开。
   * 句柄 dispose 之后调用 `show()` 什么也不做，直接 resolve。
   *
   * @returns 窗口已在屏幕上时 resolve。
   *
   * @throws `capability` manifest 没有声明 `ui.window`。
   * @throws `unavailable` 播放器在无界面模式下运行。
   * @throws `invalid` `file` 越出插件包目录。【待核实：code 取自 `plugins::package_file`】
   * @throws `missing` `file` 不存在。【待核实】
   * @throws `host` `.slint` 编译失败或组件不满足规则，`message` 是编译器的输出。
   *   【待修：`host` 不在 {@link PluginError} 错误表里，应该补进表中，或者统一成 `ui-prepare`】
   *
   * @example 打开失败时提示用户
   * ```ts
   * try {
   *   await info.show();
   * } catch (error) {
   *   void plugin.osd.show(`无法打开窗口：${(error as PluginError).code}`);
   * }
   * ```
   */
  show(): Promise<void>;
  /** 隐藏窗口；状态和属性都保留，`show()` 会原样显示回来。`opened` 仍为 `true`。 */
  hide(): Promise<void>;
  /**
   * 关闭并销毁窗口实例；句柄保留，之后 `show()` 用最新属性打开一个新实例。
   * 不会调用 {@link Window.onClose}。没有打开时调用什么也不做。
   */
  close(): Promise<void>;
  /** 窗口实例是否存在（显示或隐藏都算）。第一次 `show()` 前、`close()` 后、用户关闭后为 `false`。 */
  readonly opened: boolean;
  /**
   * 用户关闭了窗口（标题栏的关闭按钮，或标准关闭键 ⌘W / Ctrl+W / Ctrl+F4）。
   *
   * @remarks
   * 监听器被调用时 `opened` 已经是 `false`。插件自己调用 `close()` 或 `dispose()` 时不会调用它。
   * 监听器里抛出的错误交给 {@link Scope.onError}。
   *
   * @returns 用于取消监听的句柄。
   *
   * @example 关闭窗口时停止轮询
   * {@includeCode ../../examples/snippets/windows.ts#stop-polling}
   */
  onClose(listener: () => unknown): Disposable;
}
````

`examples/snippets/windows.ts` 和 `examples/snippets/ui/info.slint` 都是真实文件，CI 会编译、运行。

````ts
import type { Plugin } from "@vestra-player/sdk";

export default async function activate(plugin: Plugin) {
  // #region info-window
  const info = plugin.windows.create("ui/info.slint");
  info.bind(() => ({ source: plugin.player.state.source ?? "（无来源）" }));
  info.on("copySource", (text: string) => plugin.clipboard.writeText(text));
  plugin.commands.add({
    id: "info",
    path: ["Tools", "媒体信息…"],
    run: () => info.show(),
  });
  // #endregion info-window

  // #region stop-polling
  let timer: NodeJS.Timeout | undefined;
  const refresh = async () => info.set({ stats: JSON.stringify(await plugin.player.stats()) });
  info.on("startPolling", () => { timer ??= setInterval(refresh, 1000); });
  info.onClose(() => { clearInterval(timer); timer = undefined; });
  // #endregion stop-polling
}
````

对照原文，这次重写补上了：

- 概念定义：独立的顶层窗口、非模态、尺寸来自 Slint；
- 与面板、设置页、对话框的选型；
- 完整的生命周期，以及标准关闭键；
- 错误码（并暴露出两个需要修代码的不一致）；
- 测试方法；
- 能自成一体、可以编译的示例，以及对应的 `.slint` 文件；
- `@since`、`@category`、`@see`。

参考条目的总长度大约是原来的 4 倍。按 VS Code 的经验，更长的教学内容应该放在"概念：界面总览"和"Slint 桥接"两篇里，不要继续往注释里堆。

---

## 4. 工具链

### 4.1 `typedoc.json` 的改动

```jsonc
{
  "lang": "zh",                                  // 内置中文界面
  "projectDocuments": ["docs/**/*.md"],          // 教程、概念、操作指南；用 frontmatter 的 group/category/children 组织
  "categorizeByGroup": false,
  "navigation": { "includeCategories": true, "includeGroups": false, "includeFolders": true },
  "defaultCategory": "其他",
  "categoryOrder": ["核心", "命令与快捷键", "界面", "播放", "处理", "系统", "插件管理", "测试", "*"],
  "blockTags": ["@capability", "@player" /* 加上 TypeDoc 默认的标签 */],
  // 同时在 tsdoc.json 里声明 @capability、@player，让编辑器和 lint 都认识
  "searchInComments": true,
  "searchInDocuments": true,
  "plugin": ["typedoc-plugin-mdn-links", "typedoc-github-theme"],
  "sidebarLinks": { "错误码": "documents/错误码.html", "能力一览": "documents/能力一览.html" },
  "sort": ["source-order"]                       // 保留：源码顺序就是"常用的在前"
}
```

- 每个导出类型都加上 `@category`（约 100 处，机械性工作）。
- `@capability` 用 `blockTags` 渲染成独立小节。如果想做成徽标，可以写一个约 30 行的 `customJs`，或者写一个 TypeDoc 插件，把它渲染到签名下方。
- 自定义 `@player 0.2.0` 统一替换正文里的 "Requires player 0.2.0"。

**主题**：首选 `typedoc-github-theme`。它改动小，与 0.28 同步，外观接近 GitHub，和仓库观感一致。

**另一条路线（中期再考虑）**：用 `typedoc-plugin-markdown` 加 `typedoc-vitepress-theme`，生成 Markdown，交给 VitePress。

- 好处：
  - 手写页面的排版、目录和全文搜索更好；
  - 原生支持多语言站点（`/` 中文，`/en/` 英文）；
  - 能做"API 参考 + 指南"统一导航，最接近 VS Code 或 MDN 的体验。
- 代价：多一层构建，TypeDoc 的部分校验需要在 Markdown 输出之后再做。

建议先在 TypeDoc 默认 HTML 上完成内容重写，**内容和工具链分两步走**。

### 4.2 示例必须能编译、能运行

做三层，从易到难：

1. **`examples/` 是一个真实的 TS 工程**：
   - `examples/snippets/*.ts`，按 `// #region 名字` 切出片段；对应的 `.slint` 放在 `examples/snippets/ui/`；
   - 加一个 `tsconfig.examples.json`，在 `npm run check` 里一起跑 `tsc --noEmit`；
   - TSDoc 和 Markdown 页面一律用 `{@includeCode ../../examples/snippets/x.ts#region}` 引入。
   - 这就是 OpenCV `@snippet` 的做法。类型错误或 region 名打错，`typedoc` 都会直接报错，`treatWarningsAsErrors` 已经开启。
2. **内联代码块的抽取检查**（覆盖历史上那 79 个内联 `@example` 和 `docs/*.md` 里的代码块）：
   - 写一个 `scripts/check-examples.mjs`：
     - 用 `typedoc --json` 或 TypeScript API 遍历注释，找出 `` ```ts `` 代码块；
     - 同时扫描 `docs/**/*.md`；
     - 每个代码块写成 `.tmp/examples/<symbol>-<n>.ts`，开头注入统一的前导：
       ```ts
       import type * as SDK from "@vestra-player/sdk";
       declare const plugin: SDK.Plugin;
       ```
       必要时再加 `declare const` 补齐少量约定的变量，但要尽量消灭这类变量；
     - 最后跑 `tsc --noEmit`。
   - 有意不完整的片段用 `` ```ts nocheck `` 显式标注，数量要尽量少。
   - 这一层就是 rustdoc 的"示例即测试"（编译部分）。
3. **运行级 doctest**：对 `examples/snippets` 里每个默认导出 `activate` 的文件，在 `node --test` 里用 `startPlugin` 加 `TestHost` 激活一次，并断言几个关键的副作用（命令已注册、`host.window(id)` 存在，`userClose()` 之后轮询已经停止）。SDK 自带测试宿主，正好可以做到 Rust 的"示例会运行"。

**Slint 片段**：SDK CI 目前没有 Slint 编译器。

- 一种办法是在播放器仓库的插件测试里编译 `examples/snippets/ui/*.slint`，SDK 发版前跑一次；
- 另一种办法是引入 Slint 的 Node 包或 `slint-lsp`，只做语法检查。这一项可以放到第二期。

### 4.3 链接检查

- 站内链接：`validation.invalidLink` 已经覆盖 `{@link}`。手写 Markdown 里的相对链接，TypeDoc 0.28 也会校验，失败时作为警告，进而导致失败。
- 外链：在 `_docs.yml` 生成 `docs-site` 之后，加一步 lychee（`lycheeverse/lychee-action`）扫描 HTML，忽略 `localhost`。外链偶尔会抖动，所以建议放在定时任务（每周）里跑，PR 里只跑站内检查，以免误报阻塞合并。
- 漂移检查：一个小脚本断言 `docs/getting-started.md` 里的 Node 版本等于 `package.json` 的 `engines.node`，示例里的 SDK 版本等于当前大版本。§1.2 ⑤ 里的那类漂移就是这样被发现的。
- 覆盖率：`typedoc-plugin-coverage` 出徽章。另写一个 `scripts/doc-lint.mjs` 读 `typedoc --json`，强制执行以下规则：
  - 每个返回 `Promise` 的方法必须有 `@throws`；
  - 每个模块入口必须有 `@remarks`、`@example`、`@capability`、`@category`；
  - 每个方法必须有 `@since`。

### 4.4 文档语言

建议：**中文为主，API 参考的注释也用中文**；README 和 npm 页面保留英文简介。理由：

1. 产品、插件商城和用户都以中文为主。现有最完整的长文档（`reference.md`、`tutorial.md`、`examples.md`，约 2950 行）已经是中文，迁过来几乎不用翻译；反过来，把它们译成英文是最大的一块工作量。
2. TSDoc 的语言会直接出现在编辑器的悬停提示里。中文插件作者在 VS Code 里看到中文说明，收益最直接。
3. TypeDoc 内置 `lang: "zh"`，站点的界面文字也能统一成中文。
4. 代码标识符、错误码、能力名、Slint 关键字都保持英文。示例里的界面文字用中文，与播放器一致。

需要英文时，按投入从小到大排列：

- (a) 只维护英文的 README 和英文 getting-started（一个页面），不翻译参考；
- (b) 走 VitePress 路线，用 `/en/` 子站放英文的教程和概念页，API 参考仍然是中文 TSDoc；
- (c) 完整双语：TSDoc 写两份不现实。可行的做法是参考注释保持一种语言，另一种语言只覆盖教程、概念和操作指南。

**不建议**在同一条注释里中英混写。

如果团队判断海外开发者是重要目标：TSDoc 用英文，教程、概念和操作指南用中文，各自独立成站点区域。这样做的代价是参考和概念之间的互链会跨语言。

---

## 5. 工作量估算

| 项目 | 数量 | 说明 |
| --- | --- | --- |
| A：模块入口和核心类型，按完整模板重写（概念段、选型、生命周期、整体示例） | 约 30 个 | Plugin、Scope、Disposable、PluginError、View、Windows、Window、Panels、Panel、PanelSlot、Settings、Dialogs、Osd、Ui、Drops、MainWindow、Commands、Command、CommandOptions、Bindings、Player、Media、Conditional、Stream、Subtitles、VideoFilter、AudioFilter、VideoView、MediaCapture、Packages、Resolvers、Storage、Secrets、System、Tools，以及 testing 的 TestHost |
| B：方法，补 `@throws`、`@capability`、`@since`，以及参数的范围、单位、默认值 | 约 137 个（types.ts）+ 约 40 个（testing） | 约 90 个需要新增或改写可以编译的示例；Player、Packages、Subtitles 的一行式方法最多 |
| C：数据字段，补单位、范围、枚举值 | 约 60 个（总计约 390 个成员） | 例如 `checks: string[]`、`status: string`、`stats` 的各个字段；其余已经够用 |
| 修正事实漂移 | 约 6 处 | Node 版本、SDK 版本号、错误码表缺 `versionMismatch` / `host` / `ui-prepare`、重复窗口 id 不带 code |
| 分类标签 `@category` | 约 108 处 | 机械性工作 |
| 新写或迁移概念页 | 9 篇 | 主要来自 `reference.md` §1–3、§6、§13，以及 `tutorial.md` 的部分章节 |
| 操作指南 | 约 20 篇 | 从 `examples.md` 的 34 节合并改写，代码统一迁入 `examples/snippets` |
| 教程 | 4 篇 | 从 `tutorial.md` 和 `getting-started.md` 合并 |
| 工具链 | typedoc 配置、`tsdoc.json`、`check-examples.mjs`、`doc-lint.mjs`、示例的 doctest、lychee、主题 | 约 1–2 天 |

粗略节奏：

- 先完成工具链和"界面"这一类（Windows、Panels、Settings、View，共 8 个 A 类加上它们的方法），作为样板，交给用户评审文风。
- 然后按类别推进：播放、命令、处理、系统、插件管理、测试。每一类同时交付"概念页 + 操作指南 + 参考"。

---

## 6. 参考链接

- MATLAB `interp1`：https://www.mathworks.com/help/matlab/ref/double.interp1.html
- man-pages(7)：https://man7.org/linux/man-pages/man7/man-pages.7.html
- rustdoc 写作指南：https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html
- Rust `std::fs::File`：https://doc.rust-lang.org/std/fs/struct.File.html
- MDN `fetch()`：https://developer.mozilla.org/en-US/docs/Web/API/Window/fetch
- MDN `Window`：https://developer.mozilla.org/en-US/docs/Web/API/Window
- Stripe 创建退款：https://docs.stripe.com/api/refunds/create
- VS Code 扩展能力总览：https://code.visualstudio.com/api/extension-capabilities/overview
- VS Code Webview 指南：https://code.visualstudio.com/api/extension-guides/webview
- `vscode.d.ts`：https://github.com/microsoft/vscode/blob/main/src/vscode-dts/vscode.d.ts
- Qt `QWidget`：https://doc.qt.io/qt-6/qwidget.html
- OpenCV Canny 教程源码：https://github.com/opencv/opencv/blob/4.x/doc/tutorials/imgproc/imgtrans/canny_detector/canny_detector.markdown
- OpenCV Python 版 Canny 教程源码：https://github.com/opencv/opencv/blob/4.x/doc/py_tutorials/py_imgproc/py_canny/py_canny.markdown
- OpenCV `imgproc.hpp`：https://github.com/opencv/opencv/blob/4.x/modules/imgproc/include/opencv2/imgproc.hpp
- Django 文档首页：https://docs.djangoproject.com/en/stable/
- Diátaxis：https://diataxis.fr/
- Node.js `fs`：https://nodejs.org/api/fs.html
- TypeDoc 标签：https://typedoc.org/documents/Tags.html
- TypeDoc 输出选项：https://typedoc.org/documents/Options.Output.html
- TypeDoc 输入选项：https://typedoc.org/documents/Options.Input.html
- npm 包版本（2026-10-22 查询）：typedoc-plugin-mdn-links 5.1.1、typedoc-github-theme 0.4.0、typedoc-material-theme 1.4.1、typedoc-plugin-markdown 4.14.1、typedoc-vitepress-theme 1.1.4、typedoc-plugin-coverage 4.0.3
