# 第 3b 轮：界面组合（时间轴标记层、运行时图片、背景层、拖放、主题与外观、设置页、输入链接组件）+ 插件接入

分支：`rebuild/features-r3b`（自第 3a 轮合并到 `main` 之后的提交新建，由协调者创建）。依据：[协商记录](2026-10-features-negotiation.md)「第 3 步：裁决」（下称「裁决」）的 3b 排期、3.2 第 5 条（`ImageRef` 释放）、3.2 第 8 条、3.3「输入链接小窗」，[功能需求](../../product/feature-requirements.md)（下称「需求文档」），[社区工作流](../community-workflow.md) §3、§4、§5，[第 3a 轮任务单](2026-10-features-r3a.md)（下称「r3a」）§1、§4.1 推到本轮的项。

## 1. 范围

只做裁决里标为「3b」的条目；不提前做 4a 以后的条目，不新增裁决之外的接口。签名做不到时维护者在本文 §6.2 记录原因与调整后的签名。需求文档里比裁决宽的写法（附录 A 草案）一律以裁决为准，差异见 §4.3。

| 类别 | 条目 | 新能力 | 需求 |
| --- | --- | --- | --- |
| 时间轴 | R-M3 `plugin.timeline.marks` / `onChange` / `layer()` | `timeline.marks` | marks B22（r3a 转入）；playback 绘制 |
| 运行时图片 | R-U1 `plugin.ui.image(path): ImageRef`、`PropValue = Json \| ImageRef`；裁决 3.2 第 5 条释放规则 | — | playback I07（第 2 轮起待本轮）；visualizer J11、H12 |
| 背景层 | R-U3 `panels.mount("backdrop", file)`、`panels.backdrop.state` / `onChange` | 现有 `ui.mount` | visualizer J11、H12 |
| 拖放 | R-U2 `plugin.drops.provide`、`Panel.onDrop`（无 `onDragOver`） | `ui.drop` | playlist A15 |
| 主题与外观 | R-H6 Theme 颜色 token 宿主可设并下发；R-S2 `plugin.appearance` | `ui.theme`（`set`） | preferences H10 |
| 设置页 | R-S1 `plugin.settings.page` / `show`；R-H7 选项容器窗口 | `ui.settings` | preferences J04；hotkeys J01、J02、plugins J07、visualizer H12 的页面形态（r3a 推到本轮） |
| 宿主 UI 库 | `@vestra/widgets.slint` 新增 `UrlInputRow`（裁决 3.3，不加协议） | — | playback A02、playlist「添加链接…」 |

裁决 3.5 对本轮的统计：7 个接口 / 模块（`ui.image`、`drops.provide`、`Panel.onDrop`、`panels.mount("backdrop")`、`settings.*`、`appearance`、`timeline.*`），2 项宿主行为（H6、H7），4 个新能力名（`ui.drop`、`timeline.marks`、`ui.settings`、`ui.theme`），外加 1 个 widgets 组件。本任务单的范围与之一一对应，没有增减。

不在本轮：画中画（R-W5、R-H8、`window.state.pictureInPicture`）、显示层（R-V9）、倍帧（R-V8）、C-SIZE（R-H2）——4a；C-GPU（R-H3）、渲染选项（R-V10）、`transfer`（R-P8）——4b。已拒绝不再提：`onDragOver`（R-U2 改写）、背景层 `mode`（R-U3 改写）、`themes` 列表与 `corners`（R-S2 改写）、面板 `PopupWindow` 越界（R-U4）、`commands.popup`（R-U5）。

## 2. 两条线与顺序

| 阶段 | 执行 | 内容 | 完成条件 |
| --- | --- | --- | --- |
| P0 | 协调者 | 建分支；记录基线（§6.1）；确认 r3a 的作者交付已合并（hotkeys、plugins 两个插件存在，本轮 C 要在其上加页面） | 基线写进 §6.1 |
| P1 | 维护者 | §3 全部核心项（均为兼容扩展；H6 改 `Theme` 为 `in property` 对读取方兼容，见 §3.5） | 静态检查全过；参考文档、TestHost 同步；协调者审查通过 |
| P2 | 插件作者 | §4 分派 | 各插件 `check/build/test` 全过 |
| P3 | 协调者 | §5 验收，提交推送，合并到 `main` | — |

P1 分两批交付，作者按依赖开工（§4.2）：

| 批次 | 内容 | 解锁 | 宿主 Rust 估算 |
| --- | --- | --- | --- |
| 批次 1：视频区与数据层 | §3.1 时间轴标记层、§3.2 运行时图片、§3.3 背景层、§3.7 `UrlInputRow` | marks B22、playback I07 与标记绘制、A02 组件迁移；visualizer J11、H12；playlist「添加链接…」 | 约 1 100 |
| 批次 2：输入与界面组合 | §3.4 拖放、§3.5 Theme 下发与外观、§3.6 设置页与选项窗口 | playlist A15；preferences J04、H10；hotkeys、plugins、visualizer 的设置页 | 约 1 900 |

批次 2 内部顺序：§3.5 先于 §3.6（选项窗口是又一个要下发 Theme 的视图，复用 §3.5 的下发路径）；§3.4 与两者独立，可以穿插。

## 3. 维护者清单（P1）

每一项都要同时完成：宿主实现（`src/host/`）、SDK 类型与实现（`src/sdk/`）、TestHost 模拟（`src/sdk/plugin/testing.ts`）、参考文档（`doc/plugins/reference.md`）、单元测试；新增能力名同步 manifest 校验（宿主 `plugins.rs` 与构建工具 `scripts/plugin.mjs`）。第一方 Rust 不得用 `unsafe`。签名以裁决 3.1 / 3.3 为准，下面只写实现要点。

### 3.0 体积预算

宿主 Rust（`src/host/src/*.rs`，含测试）净增约 3 000 行。基线为 r3a 结束时的 28 577 行（P0 复测为准）。

| 项 | 宿主改动点 | 宿主 Rust 估算 | SDK / runtime 估算 |
| --- | --- | --- | --- |
| §3.1 时间轴标记层 | 新模块 `timeline.rs`（按所有者分层、校验、合并、来源变化清空）；`plugins.rs` 停用清理；事件 `timeline.changed` | 约 260 | 约 120 |
| §3.2 运行时图片 | `ui_values.rs`（`PropValue` 转换，面板与窗口两条写入路径）；新模块 `ui_image.rs`（路径校验、PNG/JPEG 头与尺寸、解码缓存、按作用域失效、LRU） | 约 500 | 约 120 |
| §3.3 背景层 | `composition.rs::shell_source` 新增 `backdrop` 槽；`host.slint` 视频区层次；可见性规则与 `panels.backdrop` 状态事件 | 约 300 | 约 80 |
| §3.4 拖放 | `main.rs` 的 winit 窗口事件（`HoveredFile` / `DroppedFile`）汇集；落点与修饰键（`NSEvent` 安全类方法）；命中面板或视频区；按扩展名的提供者表 | 约 430 | 约 100 |
| §3.5 Theme 与外观 | `theme.slint` 颜色 token 改 `in property`；两套配色表（Rust 常量）；对主窗口外壳、浮动侧栏、工具窗口、菜单视图、选项窗口逐实例下发；`ui_state.rs` 的 `appearance`；`appearance.*` 方法与事件 | 约 600 | 约 100 |
| §3.6 设置页与选项窗口 | 新模块 `settings_window.rs`（页面注册表、分类树、页面组合源码生成、切换、`show`、停用移除、`settings.lastPage`）；复用 `composition.rs` 的编译与回调转发 | 约 850 | 约 180 |
| §3.7 `UrlInputRow` | 只改 `src/host/ui/vestra/widgets.slint`（约 60 行 Slint，不计入 Rust） | 0 | — |
| 合计 | | **约 2 940** | **约 700** |

超出处理（工作流 §4.2、裁决 3.5「超过就拆」）：

- 批次 1 交付后若实际净增已超过 1 400 行，或维护者对批次 2 的复估使全轮超过 3 200 行，**把 §3.6（R-S1 + R-H7）拆出为第 3c 轮**，本轮保留 §3.4、§3.5。拆出后：preferences J04 按需求文档「过渡」形态（方案 C：「选项…」打开本插件自己的工具窗口，只含外观页）交付；hotkeys、plugins、visualizer 的设置页推迟。§3.5 不拆：H10 只依赖它，且它是 §3.6 的前置。
- 拆分决定、实测行数与理由写进 §6.2，并同步修改 §4.1 分派表。不为了压行数削减测试。

### 3.1 时间轴标记层（R-M3，能力 `timeline.marks`）

```ts
type MarkKind = "chapter" | "bookmark" | "marker";
interface TimelineMark { time: number; kind: MarkKind; title: string | null; plugin: string /* 发布者 manifest id */ }
plugin.timeline.marks: readonly TimelineMark[];                       // 同步只读；所有层合并，按 time 升序；需 playback.read
plugin.timeline.onChange(listener: (marks: readonly TimelineMark[]) => void): Disposable; // 需 events
plugin.timeline.layer(): TimelineLayer;                               // 每插件一层，第二次调用抛错；需 timeline.marks
interface TimelineLayer extends Disposable {
  set(marks: { time: number; kind: MarkKind; title?: string }[], opts?: { ifCurrent?: PlayerState }): void; // 最后写入生效；[] 清空
}
```

- 上限（需求文档 B22）：单层 ≤ 1000 个标记，`title` ≤ 100 字符；`time` 在 `[0, duration]`（时长未知时只要求 ≥ 0）；违反为 `invalid`。`set` 返回 `void`，失败交给 `onError`（与 `osd.panel().set` 一致）；`ifCurrent` 不符时丢弃本次写入，不报错。
- 来源变化（新 `sourceId`）时核心清空**所有**层；插件停用或 `dispose` 时清除该插件的层。`kind` 核心不解释，只供读取方配色。
- 变化推送新事件 `timeline.changed`（合并后的全量列表，变化才推送，至多每 100 ms 一次）。读取方没有 `events` 时 `marks` 只在启动时读一次。
- TestHost：`host.timeline`（按插件 id 记录各层、合并结果）；`host.setTimelineLayer(pluginId, marks)` 模拟别的插件发布；打开新来源时清空。

### 3.2 运行时图片（R-U1，裁决 3.2 第 5 条）

```ts
plugin.ui.image(path: string): ImageRef;   // 同步返回不透明引用；不需要能力
type PropValue = Json | ImageRef;          // View.set / bind 的属性值；只作为属性的顶层值
```

- 路径：绝对路径，位于本插件 `dataDirectory` **或本插件包目录**下；规范化后越界、经符号链接越界都拒绝。
- 校验在宿主**首次使用**该引用时进行：PNG/JPEG（按文件头，不按扩展名）、文件 ≤ 16 MB、≤ 4096×4096；失败作为属性写入错误交给 `onError`（`invalid` 或 `missing`）。
- 缓存：按（插件、规范路径、文件大小、修改时间）缓存解码结果；每插件解码内存上限 64 MB，LRU 只淘汰**当前没有被任何属性引用**的条目；被淘汰的引用仍然有效，再用时重新解码。宿主不监视文件：内容变了插件重新调用 `ui.image` 取新引用。
- 生命周期：`ImageRef` 属于创建它的作用域；作用域释放后写入该引用以 `invalid` 失败，属性里已显示的图片保持到被覆盖或视图关闭。
- wire 形式由维护者定（参考文档写明它是内部细节）。宿主对每次使用都按「该插件 + 路径规则」重新校验，所以插件绕过 SDK 伪造 wire 值也拿不到额外权限。
- 改动点：`ui_values.rs` 去掉对 image 的拒绝，`Value::Image` 只从 `ImageRef` 生成；面板（`composition.rs::set_json_global`）、工具窗口两条写入路径都要支持；`slint::Image` 从解码后的 RGBA 构造。
- TestHost：`plugin.ui.image` 返回可比较的引用（`host.images` 记录路径与所属作用域）；视图属性断言可读出引用对应的路径；作用域释放后写入得 `invalid`。

### 3.3 背景层（R-U3，现有能力 `ui.mount`）

```ts
plugin.panels.mount("backdrop", file): Panel;            // 同一时间只有一个背景层，第二个 conflict
plugin.panels.backdrop: {
  readonly state: { effective: boolean } | null;         // null：没有背景层；effective：现在是否显示
  onChange(listener: (state: { effective: boolean } | null) => void): Disposable;
};
```

- 核心只负责「没有视频帧时显示背景层」：来源阶段 `empty`，或 `ready` 且没有视频输出（`stats().video === null`）时 `effective = true`；`loading`、`error` 保持上一状态，不闪烁；有视频帧时 `false`。没有 `mode`：插件用 `player.state.phase` 与 `stats().video` 自己决定画什么（裁决 R-U3）。
- 关闭来源后视频区不再保留最后一帧（第 1 轮遗留观察）：`empty` 时宿主清空视频图像，没有背景层时为 `Theme.bg-video` 纯色。
- 层次：画在视频图像位置，铺满视频区；在宿主输入层（TouchArea）**之下**，屏幕提示与信息块**之下**；不接收指针和键盘（双击全屏、右键菜单、滚轮、拖放照常由宿主视频区处理）。
- 复用面板挂载组合（`composition.rs` 增加 `Slot::Backdrop`，首次挂载时重建外壳，同底栏、侧栏）；组件与 global 推断规则同 `panels.mount`；插件停用时卸载。
- 已知行为（裁决 3.2 第 8 条）：带封面的音频文件 mpv 会把封面当作视频轨，此时 `effective = false`，显示的是 mpv 渲染的封面。参考文档写明，不算缺陷。
- TestHost：`host.backdrop`（挂载者、`effective`）；`host.setBackdropEffective(bool)` 或随 `host.source.phase` / `host.stats.video` 推导，二选一写进参考文档。

### 3.4 拖放（R-U2，能力 `ui.drop`）

```ts
interface DropModifiers { alt: boolean; shift: boolean; cmd: boolean }
plugin.drops.provide(handler: { extensions: string[]; drop(e: { paths: string[]; modifiers: DropModifiers }): unknown }): Disposable;
Panel.onDrop(listener: (e: { paths: string[]; at: { x: number; y: number }; modifiers: DropModifiers }) => unknown): Disposable;
```

- `extensions` 小写、不带点；视频区松手时核心按扩展名把 `paths` 分组（保留 Finder 顺序），每组一次调用交给对应提供者。同一扩展名多个提供者时**先注册的生效**并写日志（与绑定冲突同规则）；没有提供者的扩展名，核心什么也不做。
- 文件夹：裁决没有写明。维护者定一个不加字段的规则并写进参考文档（建议：目录按保留名 `"folder"` 匹配，与需求文档 A15 一致），在 §6.2 记录。
- 面板（附着的底栏、侧栏，以及浮动侧栏窗口）上松手：整组交给该面板的 `onDrop`；该面板没有 `onDrop` 监听时按视频区规则分发。`at` 的坐标系在参考文档写明（作者回应按**面板坐标**设计，playlist 用自己回传的滚动位置换算到行）。工具窗口上松手：忽略。
- 不提供拖动过程中的位置（无 `onDragOver`），拖放期间不暂停、不改变播放。
- 实现要点：winit 0.30.13 只有 `HoveredFile(PathBuf)` / `DroppedFile(PathBuf)` / `HoveredFileCancelled`，每个路径一个事件，不带位置与修饰键。宿主在同一轮事件循环里收集连续的 `DroppedFile` 为一组；松手时用 `NSEvent::mouseLocation`、`NSEvent::modifierFlags`（`objc2-app-kit` 安全类方法）取位置与修饰键，换算到窗口坐标后命中面板或视频区。若实测分组或位置不可靠，在 §6.2 记录并给出降级（例如只按最后一个 `HoveredFile` 的窗口命中）。
- TestHost：`host.drop(paths, { target: "video" | "panel", at?, modifiers? })` 按同一规则分发；`host.dropProviders` 列出登记。

### 3.5 Theme 下发与外观（R-H6、R-S2，能力 `ui.theme`）

```ts
plugin.appearance: {
  readonly theme: "vestra" | "classic";              // 同步只读，不需要能力
  onChange(listener: (theme: "vestra" | "classic") => void): Disposable;
  set(theme: "vestra" | "classic"): Promise<void>;   // 需 ui.theme；其它值 invalid
};
```

- `theme.slint`：颜色 token 与 `accent-fill` 由 `out property` 改为 `in property`（默认值为现在的 Vestra 值，未下发时外观不变）；`radius-*`、字号、控件尺寸保持常量。插件 Slint 只读 `Theme`，原有写法不用改；插件不能写 `Theme`（Slint 不允许在 `.slint` 里给 global 的 `in` 属性赋值，SDK 的 `set` / `bind` 只到插件自己的 global）——加一条编译测试断言。
- 配色表放宿主（Rust 常量，两套各列全部颜色 token）：`vestra` = 现值；`classic` = [ui-alignment](../../product/ui-alignment.md) §2.1 的 经典配色（强调 `#fae100`、正在播放 `#2282f2`、底色 `#1e1e1e` 等），`accent-fill` 用纯色 `#fae100`。
- 下发：每个视图实例（主窗口外壳含底栏与侧栏、浮动侧栏、工具窗口、菜单视图、背景层、选项窗口）创建时与配色变化时 `set_global_property("Theme", …)`；单个视图失败不影响其它视图，下次刷新重试；切换时不中断正在拖动的滑块与打开的菜单。
- 选择由核心持久化：`ui-state.json` 的 `appearance.theme`（去抖保存）；preferences 停用后配色保持。变化推送新事件 `appearance.changed`，推送规则同 `window.onChange`（有 `events` 时跟随）。
- 屏幕提示「配色：…」由调用方插件自己显示，核心不显示。
- TestHost：`host.appearance`（当前值）、`host.setAppearance(theme)` 模拟别的插件切换。

### 3.6 设置页与选项窗口（R-S1、R-H7，能力 `ui.settings`）

```ts
type SettingsCategory = "general" | "playback" | "subtitles" | "video" | "audio" | "input" | "appearance" | "plugins" | "other";
plugin.settings.page(file: string, options: { id: string; category: SettingsCategory; title: string; order?: number }):
  View & Disposable & { readonly ready: Promise<void> };
plugin.settings.show(pageId?: string): Promise<void>;   // 只能指定本插件的页面；省略 = 上次的页面
```

- 页面文件规则、组件与 global 推断同 `panels.mount`，**不加** `component` / `global` 选项，没有 `visible`（裁决 R-S1）。`id` 插件内唯一（`missing` / `invalid` 规则写进参考文档）；`title` 1–24 字符；每插件 ≤ 8 页。回调没有「先绑定再显示」的限制。
- 容器窗口（H7）由核心创建与拥有，标题「选项」：640×480、最小 600×420、可调整大小；左侧分类树 160 宽（九类固定顺序：常规 / 播放 / 字幕 / 视频 / 声音 / 快捷键与鼠标 / 外观 / 插件 / 其它；空分类不显示；同一分类内按 `order`、再按插件 id 排序）；右侧页面区固定 440 宽，超高时垂直滚动；底部只有「关闭」。几何与颜色按需求文档 J04「界面」。
- 行为：再次 `show()` 把窗口带到前面并切页；`show(别的插件的页面)` 为 `invalid`；`settings.lastPage` 存 `ui-state.json`，页面不存在时显示第一页；页面所属插件停用时移除该页（正在显示则切到同分类第一页，分类空了切到「常规」第一页，全空显示「没有可设置的项目」），重新启用后随注册恢复；⌘W 关闭；窗口内快捷键不触发（同工具窗口）；打开不影响播放。
- 组合：复用 `composition.rs` 的编译与回调转发，每个页面一个所有者 global（`global_alias`），属性写入与回调只连到注册它的插件。页面集合变化时窗口重建，页面状态由各插件的 `bind` 恢复。
- TestHost：`host.settings`（已注册页面、`show` 调用记录、当前页）；页面作为 `View` 可断言属性与触发回调。

### 3.7 输入链接组件 `UrlInputRow`（裁决 3.3，不加协议）

- 在 `src/host/ui/vestra/widgets.slint` 新增 `export component UrlInputRow`：输入框 + 「确定」「取消」，规格按 [ui-alignment](../../product/ui-alignment.md) UI-DLG-02（不做历史列表）。建议接口：`in-out property <string> text`、`in property <string> placeholder`、`in property <string> confirm-text`（默认「打开」）、`in property <bool> enabled`、`callback accepted(string)`（回车或点确定）、`callback cancelled()`（点取消或 Esc）。
- 只用 `Theme` token，不含业务（不校验 URL，校验由插件做）。
- 参考文档 §6.4 的控件列表加上它；示例里给一个 ≤ 15 行的用法。

### 3.8 文档

- 参考文档：上述全部接口；§2 能力表加 `ui.drop`、`timeline.marks`、`ui.settings`、`ui.theme`，`ui.mount` 一行加 `"backdrop"`；§6.1 `PropValue` 与 `ImageRef`（释放规则）；§6.3 背景层与 `Panel.onDrop`；§6.4 `Theme` 改为宿主下发（插件只读）、`UrlInputRow`；新增小节：时间轴标记层、拖放、外观、设置页；§14 TestHost 新增项；§17 本轮变更说明（`Theme` 由常量改为宿主下发属于默认行为变化，写明对插件无需改动）。
- 示例：`ui.image` 显示图片、`timeline.layer().set`、`settings.page` + `show`、`drops.provide` 各一个短例（≤ 30 行）。

## 4. 插件接入分派（P2）

作者规则同第 2 轮（[2026-10-features-r2.md](2026-10-features-r2.md) §4）与 r3a §4：只写自己的插件目录；按需求文档对应小节实现；缺核心能力时按工作流 §4.1 写请求，同时继续不依赖它的部分；本轮核心提供了的自制实现必须删除并改用核心能力，报告列出删除的文件与行数。降级项按裁决实现，同时修改需求文档对应小节（§4.3，协调者合并时审）。

### 4.1 分派表

| 作者 | 插件 | 新功能（编号 → 依赖） | 迁移到核心能力 | 降级与文档 | 工作量 |
| --- | --- | --- | --- | --- | --- |
| A | playback | I07 进度条悬停缩略图（`ui.image` + `tools.run("ffmpeg")`，缓存在 `dataDirectory/thumbs/`，提示按 R-U4 用负 `y` 画在视频区）；B22 绘制侧：进度线上画 `timeline.marks`（章节 `Theme.text-dim`、书签 `Theme.accent`）与 `state.loop` 区间（`Theme.playing`），悬停标记 ±3px 时提示「标题  01:23」并与 I06 合并 | A02「打开链接…」窗口改用 `UrlInputRow`（`ui/open-url.slint` 中的自绘输入行删除） | 无 | 中高（批次 1） |
| B | marks、playlist | marks：B22 发布侧（`timeline.layer()`；「在进度条上显示标记」开关，`marks.json` 的 `showOnSeekbar`；章节 ∪ 书签，带 `ifCurrent`，超 1000 截断）。playlist：A15 拖放（`drops.provide` 登记媒体扩展名、列表文件、文件夹；视频区加入并播放第一项，Option 只加入；`Panel.onDrop` 插到落点所在行之前） | playlist「添加链接…」（`ui/link.slint`）改用 `UrlInputRow`；`ui/sidebar.slint` 的 2 处硬编码颜色改用 `Theme` token（H10 验收要求插件 Slint 无硬编码颜色） | A15 按 §4.3 改写需求文档 | 中（marks 批次 1；playlist 拖放批次 2） |
| C | hotkeys、plugins | hotkeys：J01、J02 的设置页形态（`settings.page`，分类 `input`，id `hotkeys.keyboard` / `hotkeys.mouse`；「快捷键…」「鼠标…」改为 `settings.show(…)`；J01 按 440 宽重排：命令 220、快捷键 140、状态 60）。plugins：J07 的设置页形态（分类 `plugins`，「插件管理…」改为 `settings.show`） | 两插件原有工具窗口：页面与窗口内容相同，作者决定保留窗口还是只留页面，写进 README | 若 §3.6 拆到 3c（§3.0），本行整体推迟 | 小中（批次 2） |
| D | visualizer（新） | J11 纯音频专辑封面（`panels.mount("backdrop")` + `ui.image`；封面用 `tools.run("ffmpeg"/"ffprobe")` 提取，或同文件夹 `cover.jpg` 等；标签 ffprobe；缓存 `dataDirectory/cache/covers/`）；H12 空闲徽标（内置 logo 用 `@image-url`；自定义徽标 `dialogs.openFile` → 复制到 `dataDirectory/logo.<ext>` → `ui.image`）；「徽标与专辑封面」设置页（`settings.page`，分类 `appearance`） | — | 无 `mode`：用 `phase` 与 `stats().video` 判断（§4.3）；README 记封面被 mpv 当作视频轨的已知行为 | 高（批次 1；设置页批次 2） |
| E | preferences（新） | J04 首选项：「设置 ▶ 选项…」（F5，order 830）→ `settings.show()`；本插件「外观」页（`settings.page`，分类 `appearance`）。H10 配色：「设置 ▶ 外观 ▶ Vestra / 经典」（radio，840、841）+ 外观页的配色单选与 120×40 预览块（自己用 Slint 画）+「恢复默认」；`appearance.set` / `onChange`；切换时 OSD「配色：…」 | — | 去掉圆角选项与 `themes` 列表（§4.3）；若 §3.6 拆到 3c，J04 按需求文档「过渡」形态（自己的工具窗口，只含外观页） | 中（批次 2） |

不在本轮的插件条目：window H07（4a）；video E11、E13、E16–E18（4a / 4b）。subtitles「字幕设置…」改为设置页属于需求文档的「以后可以」，本轮不要求。youtube、bilibili 的链接输入小窗（UI-DLG-02 的同一规格）可以顺带改用 `UrlInputRow`（不强制，报告说明）。其余插件没有本轮条目；H10 验收若发现别的插件 Slint 有硬编码颜色，按缺陷退回该插件作者。

### 4.2 准入

- 批次 1（§3.1–§3.3、§3.7）审查通过后：A、B（先做 marks B22 与 playlist 的 `UrlInputRow`、颜色修正）、D 开工。
- 批次 2（§3.4–§3.6）审查通过后：B 接着做 playlist A15；有空位时依次开工 E（preferences，J04 的验收依赖它）、C（hotkeys、plugins 页面）；D 补设置页。
- C 的前提：r3a 的 hotkeys、plugins 作者交付已合并（P0 确认）。
- 作者不等依赖的部分可以先做：界面、菜单、设置保存、TestHost 下的测试（例如 E 先做菜单 radio 与外观页布局，D 先做封面提取与缓存）。
- 同时运行的作者不超过 3 个（含协调者共 4 个）。建议顺序：A、B、D →（A 完成后）E →（下一个空位）C。

### 4.3 需求文档按裁决改写（作者各改自己的章节）

| 章节 | 需求文档现写法 | 按裁决改为 |
| --- | --- | --- |
| playlist A15 | `kinds`；`onDragOver` 画插入线；同扩展名 `conflict`；无处理者时核心 OSD「无法打开拖入的文件」 | `extensions`；无插入线，插到落点所在行之前、空白处追加；先注册的生效；无处理者时核心不做任何事（验收项改为「列表不变、无提示」） |
| visualizer J11、H12 | `backdrop.state.mode: "idle" \| "audio"` | 只有 `effective`；插件用 `phase` 与 `stats().video` 区分 |
| preferences J04 | `component?`、`global?` 选项；`SettingsPage.visible` | 去掉；推断规则同 `panels.mount` |
| preferences H10 | `themes`、`ThemeInfo`、`corners`；圆角选项；`ui-state.json` 的 `corners` | 固定两个 id；圆角跟随配色、不可单独改；只存 `appearance.theme` |
| playback I07 | 路径只在 `dataDirectory` 下 | `dataDirectory` 或本插件包目录（缩略图仍放 `dataDirectory`，行为不变） |
| hotkeys J01 | 页面 600×460 | 设置页 440 宽（命令 220、快捷键 140、状态 60）；工具窗口形态若保留仍按原尺寸 |

## 5. 验收

### 5.1 静态检查与测试

工作流 §6.1 全部命令；各插件测试数不减少；新插件（visualizer、preferences）`check/build/test` 通过；菜单快照只多出本轮新增项（`doc/process/rounds/menus-snapshot.test.ts`）；插件 `.slint` 中（主题文件除外）grep 硬编码颜色 `#[0-9a-fA-F]{6}` 结果为 0。

### 5.2 真实 GUI 测试（协调者）

工作流 §6.2 的全部必测场景，加上本轮：

| 范围 | 场景 | 证据 |
| --- | --- | --- |
| 时间轴标记 | 5 章 + 2 书签的 mkv：进度条 7 条竖线，书签为强调色，悬停书签显示标题与时间；关闭「在进度条上显示标记」1 秒内消失、再开恢复；切到无章节文件旧标记立即消失；设 A-B 循环后区间与 A、B 竖线可见；停用 marks 后无标记、A-B 区间仍在 | 截图 |
| 运行时图片 / I07 | 开启悬停缩略图，1080p 本地 mkv 上移动：200 ms 内出现缩略图且与该时间画面一致；同位置再悬停立即显示；YouTube 来源只显示时间；移走 ffmpeg 后菜单灰；快速来回 10 秒 `ps` 中至多 1 个 ffmpeg；长时间悬停宿主内存不持续上涨（活动监视器前后对比） | 截图、`ps`、内存读数 |
| 背景层 / J11 | 带内嵌封面的 MP3 显示封面与标题 / 艺术家 / 专辑；无封面的 FLAC 文件夹有 `cover.jpg` 时显示它；无封面时半透明 logo 与文件名；双击仍全屏、右键仍出菜单、滚轮仍调音量；音频切到视频 1 秒内显示视频画面；关闭开关为纯色；停用 visualizer 后纯色且播放不中断 | 截图 |
| 背景层 / H12 | 无参数启动显示 logo 与「Vestra」；打开文件后消失，F4 关闭后重新出现（不残留最后一帧）；窗口拖到最小 logo 64px 不溢出、全屏 256px；自定义 PNG 立即替换，删原文件并重启后仍在，「恢复默认徽标」回到品牌 logo；空闲时双击切换全屏 | 截图 |
| 拖放 / A15 | 从 Finder 拖 1 个 mkv 到视频区：加入并播放；拖文件夹：按 A03 展开并播放第一项；按住 Option 拖 3 个文件：加入 3 行不播放；拖 2 个文件到列表第 3、4 行之间：插在落点行之前、不播放；拖 `.m3u8`：解析并播放第一项；拖 `.srt` 且无插件登记：无反应、列表不变；侧栏浮动时拖到浮动窗口与附着时相同；侧栏因空间隐藏时拖到视频区仍有效 | 物理拖放记录、截图 |
| 主题 / H10 | 切到「经典」1 秒内：底栏进度条变黄、菜单选中项变色、已打开的音频工作台窗口与浮动侧栏同步变色；拖动音量滑块时切换，滑块不中断；重启后保持；停用 preferences 后配色不变；切回 Vestra 后与本轮开始前的截图一致 | 截图 |
| 设置页 / J04 | F5 打开「选项」，分类树至少有「外观」（preferences、visualizer）、「快捷键与鼠标」（hotkeys）、「插件」（plugins）；点页面右侧切换，播放不受影响；停用 hotkeys 后该分类 1 秒内消失，重新启用后恢复；在外观页切换配色，选项窗口自身与主窗口立即变化；关闭再开显示上次页面；⌘W 关闭；窗口内按 Space 不触发播放/暂停；「快捷键…」直接打开对应页面 | 截图 |
| 输入链接组件 | playback「打开链接…」与 playlist「添加链接…」外观一致（同一组件）；回车 = 确定、Esc = 取消；两者功能与改动前一致 | 截图 |

若 §3.6 拆到 3c，本表「设置页」一行改为过渡形态：F5 打开 preferences 自己的窗口，包含外观页。

缺陷归属同工作流 §6.2。

## 6. 记录

### 6.1 基线

（P0 填写：静态检查、各插件测试数；宿主 Rust `src/host/src/*.rs` 行数——撰写本任务单时为 28 577；SDK 与运行器行数）

### 6.2 维护者交付与签名调整

（P1 每批交付时填写：实现清单、签名调整与实现取舍、检查、宿主 Rust 实测净增、给协调者的 GUI 实测要点。至少要记录：拖放文件夹的匹配规则、`Panel.onDrop` 的坐标系、拖放分组与落点的实测结论、`ImageRef` 的 wire 形式、是否触发 §3.0 的拆分）

#### P1 批次 1（2026-10-21，维护者）：时间轴标记层、运行时图片、背景层、`UrlInputRow`

基于 `main` `b16f4cf`（第 3a 轮合并后，含按键修复）。

**实现清单**（每项：宿主 + SDK 类型与封装 + TestHost + 参考文档 + 单元测试）

| 任务单 | 内容 | 宿主 | SDK / TestHost / 文档 |
| --- | --- | --- | --- |
| §3.1 | `plugin.timeline`（R-M3，能力 `timeline.marks`） | 新模块 `timeline.rs`：按所有者分层、校验（≤ 1000、标题 ≤ 100 字符、`[0, 时长]`、三种 `kind`）、按时间稳定合并；`timeline.marks`（`playback.read`）、`timeline.set`（`timeline.marks`）；`source.state` 广播时比较来源代数，变了清空所有层；`disable` 清除该插件的层；新事件 `timeline.changed`（修订号变化且距上次 ≥ 100 ms 才推） | `sdk/plugin/timeline.ts`：有 `playback.read` 时激活前读一次，第一次读 `marks` / `onChange` 后跟随事件；`layer()` 第二次抛错，`set` 用 `Latest`（最后写入生效）、失败交 `onError`，`dispose` 写 `[]`；`host.timeline`、`host.setTimelineLayer`；参考 §5.6 |
| §3.2 | `plugin.ui.image`、`PropValue`（R-U1，裁决 3.2 第 5 条） | 新模块 `ui_image.rs`：路径（绝对、规范化后在包目录或数据目录内，含符号链接）、≤ 16 MB、PNG/JPEG 文件头、`image` crate 先读尺寸（≤ 4096×4096）再解码为 RGBA；缓存键（插件、规范路径、大小、修改时间），每插件 64 MB，LRU 只淘汰没有属性在显示的条目（按「插件 + 视图 + 属性」记录正在显示的键）；`Host::prop_value` 是全部写入路径的唯一入口：`ui.open` 初值、`ui.set`、`ui.setMount`（主窗口与浮动侧栏）、外壳重建与浮动窗口打开时的重放；`ui_values::reconcile` 对图片总是重新安装 | `ImageHandle`（`ui.ts`）；`ViewBase` 发送前转线上形式，作用域已释放时 `invalid`（初值里的失效图片被略去并报告）；`Scope.ui`；`host.images`、`host.imagePath`；参考 §6.1 |
| §3.3 | `panels.mount("backdrop")`、`panels.backdrop`（R-U3） | `composition::Slot::Backdrop`，`host.slint` 新标记 `VESTRA-MOUNT-BACKDROP` 在视频图像之后、`mouse` TouchArea 之前（组合测试断言顺序）；新属性 `no-video`（视频图像 `visible: !no-video`，背景层 `visible: no-video`）；`composition::no_video` 状态规则；`source.state` 新内部字段 `videoOutput`（mpv 有选中的视频轨，含封面）；`empty` 时清空视频图像；第二个背景层 `conflict`（含队列中的挂载）；背景层不进窗口策略（无偏好、无自动隐藏，`setInteraction` 无效、`showMenu` `unavailable`） | `layout.state` 与 `window.state` 多 `backdrop: { owner, effective }`；`LayoutService` 带上它；`host.backdrop`、`host.setBackdropEffective(bool)`（二选一选了显式设置）；参考 §6.3 |
| §3.7 | `UrlInputRow` | `ui/vestra/widgets.slint`（约 55 行 Slint，放在文件末尾：Slint 要求先声明 `Button`）；`ui.rs` 的库编译测试用到它 | 参考 §6.4；示例 §29 |
| — | §4.3 请求：确认框挂调用方窗口 | 见下「请求裁决」：`confirm.rs` 改挂 `NSApplication.keyWindow`（安全方法，可见且没有已挂表单时），否则主窗口 | 参考 §7、§17 |
| §3.8 | 文档 | — | 参考 §2、§3.3、§5.6（新）、§6.1、§6.3、§6.4、§7、§14、§17 批次 1；示例 §27–§29 |

**签名调整与实现取舍**（签名均与裁决一致）

1. **`ImageRef` 的 wire 形式**：`{ "$vestraImage": "<路径>", "ref": <序号> }`，SDK 的 `ImageHandle.toJSON()` 生成。`ref` 让每次 `ui.image` 调用都是新值：否则同一路径的两次调用 JSON 相同，`bind` 的「变化才发送」会吞掉「文件内容变了、重新取引用」这一步。宿主只认恰好这两个键的对象，并且每次使用都重新检查路径，伪造的 wire 值拿不到额外权限。`ImageRef` 类型带 `path`（只读，测试与日志用）和品牌字段 `__vestraImage`。
2. **作用域释放**由 SDK 判断（宿主看不到 SDK 作用域）：`ImageHandle` 记住创建它的作用域，写入时作用域已释放则 `invalid`，请求不发出。所以 TestHost 的 `host.images` 只记录写入过的图片（路径与序号），不记录作用域；任务单 §3.2 的「`host.images` 记录路径与所属作用域」按此收窄。
3. **`ifCurrent` 在 `timeline.layer().set` 里只比较来源（`sourceId`）**，不比较整个 `PlayerState` 戳：按其它控制的规则，读章节期间用户暂停一下，标记就会被静默丢弃且不再重试，对标记层没有意义（标记属于来源，不属于暂停状态）。线上为 `timeline.set { marks, sourceId? }`，返回 `{ applied }`（SDK 不对外暴露）。
4. `timeline.marks` 在激活前读一次（只对有 `playback.read` 的插件），与 `player.state`、`window.state` 并行，让 `activate` 里同步读到真实值；参考 §17 已写明协议计数测试会看到它。
5. 背景层的「没有视频输出」用 mpv 的 track-list（有选中的视频轨，**含**封面）判断，与 `stats().video !== null` 一致，满足裁决 3.2 第 8 条（封面音频 `effective = false`）；生成流（NES）就绪即算有画面。`ended` 与 `ready` 同规则（keep-open 时最后一帧仍在）。启动时（还没有来源）为 `true`。
6. 关闭来源后清空视频图像只在阶段变为 `empty` 时做；纯音频就绪时不清空，而是靠 `no-video` 隐藏视频图像，所以从视频切到音频也不会残留上一帧。
7. 每个插件仍只能挂载一个面板（宿主与 SDK 原有限制，参考 §6.3 补了一句）。visualizer 只挂背景层，不受影响；以后有插件要同时挂底栏与背景层再议。
8. 图片解码在 UI 线程同步进行（4096² JPEG 约 0.1 s）；缩略图（I07）这类小图无感。缓存命中不读文件内容，只 `stat` 一次。
9. 新依赖：`image = "=0.25.10"`（`default-features = false`，`png`、`jpeg`），Slint 已经依赖同一版本，`Cargo.lock` 只多一行依赖关系，不新增 crate。

**请求裁决：`dialogs.confirm` 挂到调用方工具窗口**（协调者转来，§4.3；请求者 plugins、hotkeys，以后 preferences）

- 请求：确认框固定挂主窗口表单，从工具窗口调用时主窗口被提到前面、工具窗口被盖住；希望加选项 `{ window?: Window }`。插件目前在确认后 `show()` 自己的窗口规避。
- 维护者分析：这是现有实现的缺陷（确认框的父窗口应是用户正在操作的窗口），按 §4.2 第 1 条按缺陷修，不加参数。调用确认的工具窗口在用户点按钮时就是应用的活动窗口（key window），宿主改为挂 `NSApplication.keyWindow`（可见、没有已挂的表单时），没有活动窗口时挂主窗口，再没有为应用模态。这样不需要宿主把 SDK 的 `Window` 句柄映射到原生窗口（工具窗口目前没有原生窗口关联，要做需在 `ui.open` 前后各拍一次窗口快照，约 40 行，且多一个参数要维护）。
- 代价：插件在后台（计时器里、用户正在别的窗口）弹确认框时，它挂在用户当前的窗口上而不是插件的窗口上——对用户而言仍是合理的位置；需要明确父窗口的场景目前没有。
- 结论：**改写后接受**（不加 `window` 选项，宿主行为修正）。签名不变；参考 §7 与 §17 已更新。plugins、hotkeys 的「确认后 `show()` 自己」规避可以删除（作者在本轮顺带处理，不强制）。

**检查**（worktree 内实际执行）

- `src/`：`check`、`format:check`、`boundaries`、`plugins:build` 0；`test:sdk` 84/84（新增 `tests/sdk/round3b.test.ts` 3 条：时间轴、运行时图片、背景层）；插件测试（`plugin -- test --all`）audio 35、bilibili 15、capture 25、history 22、hotkeys 17、info 26、marks 20、nes 11、playback 45、playlist 62、plugins 10、subtitle-online 13、subtitles 52、video 22、window 17、workshop 17、youtube 14 全过（workshop `vendor/sdk` 已重同步，同一提交）。
- `src/host/`：`cargo fmt --check`、`clippy --locked --all-targets -D warnings`（含 `--no-default-features`）0；`cargo test --locked` 202/202（新增：标记层合并与推送 1、标记校验 1、图片引用形式 1、图片路径/格式/尺寸 1、图片缓存与淘汰 1、背景层状态规则 1；组合编译测试加背景层 fixture 与层次断言，库编译测试加 `UrlInputRow`）。
- 未执行：GUI（按要求不操作）。
- 宿主 Rust：`src/host/src/*.rs` 28 771（`main` `b16f4cf`）→ 29 597（**+826**，含测试；`timeline.rs` 241、`ui_image.rs` 345）。未超过 §3.0 的 1 400 行门槛；批次 2 复估约 1 700 行，全轮约 2 500，**不触发 §3.6 拆分**（批次 2 交付时按实测再确认）。SDK 与运行器（`src/sdk/plugin/*.ts` + `src/runtime/`）+约 470（含 TestHost +140），`src/sdk/wire.ts` +28。

**给协调者的 GUI 实测要点**（需要一个调用这些接口的插件：作者 B 的 marks、D 的 visualizer、A 的 playback，或临时探针）

1. 时间轴：探针发布 3 个标记后 `timeline.marks` 立即包含它们；打开另一个文件后为空（`timeline.changed` 推送空列表）；停用发布者后它的标记消失、其它插件的标记保留。
2. 运行时图片：数据目录里的 PNG 与 JPEG 都能显示（工具窗口与背景层各一次）；把 `.png` 改名成 `.jpg` 仍能显示（按内容判断）；数据目录外的文件、符号链接指向外部、5000×5000 的图、GIF 都失败并进 `onError`，属性不变；覆盖同名文件后再调用 `ui.image` 显示新内容；侧栏浮动/附着切换、底栏重建后图片仍在。
3. 背景层：无参数启动视频区显示背景层；打开视频后消失、看到视频；F4 关闭后背景层重新出现、**没有残留最后一帧**；没有背景层插件时 F4 后为纯色；带内嵌封面的 MP3：背景层不显示（mpv 画封面）、无封面的 FLAC：背景层显示；背景层显示时双击仍全屏、右键仍出菜单、滚轮仍调音量；从视频直接切到无封面音频不闪最后一帧。第二个插件挂背景层得 `conflict`。
4. `UrlInputRow`：回车 = 确定、Shift+回车换行、Esc = 取消；`enabled: false` 时确定灰、回车无效；颜色随 `Theme`。
5. 确认框：从工具窗口（插件管理「卸载」、快捷键设置「恢复默认」）触发时，表单挂在该工具窗口上，主窗口不被提到前面；从主窗口菜单触发时仍挂主窗口。

#### P1 批次 2（2026-10-21，维护者）：拖放、Theme 下发与外观、设置页与选项窗口

基于 `rebuild/features-r3b`（批次 1 `82862f4`）。**未触发 §3.0 的拆分**：§3.6 在本批完成（实测见「检查」）。

**实现清单**

| 任务单 | 内容 | 宿主 | SDK / TestHost / 文档 |
| --- | --- | --- | --- |
| §3.5 | Theme 下发（R-H6） | 新文件 `ui/palette.slint`（宿主专用 global `VestraPalette`，颜色与 `accent-fill` 为 `in` 属性，默认 Vestra）；`ui/vestra/theme.slint` 的颜色改为绑定到它的 `out` 属性；新模块 `theme.rs`：两套配色表（21 个颜色 token；参考播放器的 `accent-fill` 为纯色 `#fae100`），`apply` 用 `set_global_property("VestraPalette", …)`；外壳（`host.slint` 导出 palette）、浮动侧栏、工具窗口（`ui.rs` 在源码末尾追加 `export { VestraPalette } from <绝对路径>`，显示前下发）、选项窗口逐实例下发；失败置 `theme_pending`，下一个 16 ms 周期全部重发 | 参考 §6.4、§6.8、§17 |
| §3.5 | `plugin.appearance`（R-S2，能力 `ui.theme`） | `appearance.set`（`invalid` 校验）；`ui-state.json` 的 `appearance.theme`（去抖保存，读坏的值丢弃）；变化时广播 `appearance.changed`；`window.state` 答复带 `appearance` | `sdk/plugin/appearance.ts`：初值随激活前的 `window.state` 读取（不加请求），第一次读 `theme` / `onChange` 后跟随事件；`host.appearance`、`host.setAppearance` |
| §3.6 | `plugin.settings`（R-S1，能力 `ui.settings`）与选项窗口（R-H7） | 新模块 `settings_window.rs`：页面表（校验 id、分类、标题 1–24、`order`、每插件 ≤ 8、注册时单独编译检查）、九类固定顺序的树、窗口源码生成（每页 `Page<序号>View` 与 `Settings<序号>Bridge` 别名，`if current == k` 实例化，右侧 `ScrollView` 440 宽）、属性写入复用 `prop_value`（图片可用）与 `reconcile`、回调经 `ui.callback`（视图 `$settings:<id>`）只发给注册它的插件；`show` 打开或用 winit `focus_window` 带到前面；`settings.lastPage`；停用 / `dispose` 移除并在下一周期重建，按「同分类 → 常规 → 第一页 → 空」回退；⌘W、关闭按钮、标题栏关闭都是隐藏并丢弃实例 | `sdk/plugin/settings.ts`（`ViewBase` 子类，组件与 global 推断复用 `exported`）；`host.settings`、`host.settingsPage`；参考 §6.9；示例 §30 |
| §3.4 | `plugin.drops.provide`、`Panel.onDrop`（R-U2，能力 `ui.drop`） | 新模块 `drops.rs`：提供者表（先登记生效，冲突写日志）、按扩展名分组、`folder`、30 ms 无新路径即一组；winit `DroppedFile` 在外壳与浮动侧栏的窗口钩子里转为事件；松手时 `platform_window::pointer_in`（`NSWindow::mouseLocationOutsideOfEventStream` + `contentRectForFrameRect`）与 `modifier_keys`（`NSEvent::modifierFlags` 类方法），都是 objc2 安全方法；`drops::hit` 按视频列宽高判定底栏 / 侧栏并换算面板坐标；浮动侧栏打开时关联原生窗口 | `sdk/plugin/drops.ts`（提供者 id 由 SDK 生成；面板监听计数，第一个 `onDrop` 发 `drops.listen(true)`，最后一个撤销）；`host.drop`、`host.dropProviders`、`host.panelDrops`；参考 §6.7；示例 §31 |
| §3.8 | 文档 | — | 参考 §2、§3.3、§6.3、§6.4、§6.7–§6.9（新）、§14、§17 批次 2；示例 §30、§31 |

**签名调整与实现取舍**（公开签名均与裁决一致）

1. **Theme 的「插件不可写」换了实现方式**：任务单 §3.5 假设「Slint 不允许在 `.slint` 里给 global 的 `in` 属性赋值」，实测 Slint 1.16 允许（`Theme.accent = #ff0000` 编译无诊断）。改为：可写的颜色放在宿主专用的 `VestraPalette`（`ui/palette.slint`，不在 `@vestra` 库目录里），`Theme` 的颜色仍是 `out` 属性、绑定到它。插件照旧 `import { Theme }`，赋值 `Theme.x` 编译失败（`theme.rs` 的编译测试断言）。插件理论上可以按绝对路径导入 palette 自己改色，这属于有意绕过（插件是受信任的，见参考开头）。
2. 配色表中参考播放器没有对应项的 `selected-bg` 取 `#444444`（按钮按下灰），`button-hover` / `button-press` / `button-disabled` 取 ui-alignment §2.1 注释里的值。Vestra 的 `accent-fill` 渐变从第一个实例读出缓存（Slint 公开 API 不能构造渐变），切回 Vestra 时下发它。
3. `appearance.theme` 的初值随激活前已有的 `window.state` 读取返回，不增加协议请求；事件 `appearance.changed` 按任务单新增，需要 `events`。
4. **拖放文件夹规则**：目录（按松手时 `is_dir`）匹配保留名 `"folder"`，不展开；没有扩展名的文件不匹配任何提供者。
5. **`Panel.onDrop` 的坐标系**：面板坐标 = 相对面板根组件左上角的逻辑点（底栏：`y − 视频列高`；侧栏：`x − 视频列宽 − 3`；浮动侧栏：窗口内容坐标）。
6. **拖放分组与落点**：winit 0.30 对一次拖放的多个文件在同一批事件里逐个发 `DroppedFile`，宿主以「30 ms 没有新路径」结束一组；位置不取 winit（拖动期间没有 `CursorMoved`），而在结束时问 AppKit（`mouseLocationOutsideOfEventStream`，此时指针就在松手处）。**以上未做 GUI 实测**（按要求不操作 GUI），列入 GUI 要点。若实测不可靠的降级：按窗口命中（落在浮动侧栏窗口 = 面板，主窗口 = 视频区规则）。全屏时底栏浮在视频上，本批一律按视频区处理（记入参考 §6.7）。
7. 面板没有 `onDrop` 监听时按视频区规则分发（裁决原文）；监听以插件为单位（每插件只有一个面板）。
8. `settings.show(pageId)`：`pageId` 不是本插件已注册的页面一律 `invalid`（裁决只写了「别的插件的页面」；插件看不到别人的 id，所以本插件不存在的 id 同样处理）。重复 id、超过 8 页、标题与分类不合法也是 `invalid`。
9. 选项窗口每次页面集合变化都重建（全部页面编在一个实例里，切页只改 `current`，不重建）；重建保持窗口位置大小，属性按最后写入值重放，图片经缓存。页面注册时单独编译一次，坏页面让自己的 `ready` 失败（`ui-prepare`），不会拖垮窗口。
10. 关闭选项窗口丢弃实例（下次 `show` 重建），页面状态由插件的 `bind` 与播放器记录的属性恢复。

**检查**（worktree 内实际执行）

- `src/`：`check`、`format:check`、`boundaries`、`plugins:build` 0；`test:sdk` 87/87（新增 `tests/sdk/round3b-input.test.ts` 3 条：拖放、外观、设置页）；插件测试（合并 `eda77b8` 作者批次 1 之后，`plugin -- test --all`）audio 35、bilibili 15、capture 25、history 22、hotkeys 17、info 26、marks 24、nes 11、playback 59、playlist 63、plugins 10、subtitle-online 13、subtitles 52、video 22、window 17、workshop 17、youtube 14 全过（workshop `vendor/sdk` 已重同步，同一提交）。第一次全量运行 playback 的「I07 a resting pointer makes a picture once…」超时失败 1 次（并行负载下的计时用例，单独连跑 3 次与第二次全量均通过），转作者 A 留意。
- `src/host/`：`cargo fmt --check`、`clippy --locked --all-targets -D warnings`（含 `--no-default-features`）0；`cargo test --locked` 208/208（批次 2 新增：配色表与 palette 文件一致 1、插件不能给 Theme 赋值且导出可编译 1、设置树排序与回退 1、选项窗口源码编译（有页 / 无页）1、拖放分组与提供者顺序 1、拖放成组与面板命中 1）。
- 未执行：GUI。
- 宿主 Rust：`src/host/src/*.rs` 29 597 → 30 966（批次 2 **+1 369**，`settings_window.rs` 633、`drops.rs` 344、`theme.rs` 234）；本轮累计 28 771 → 30 966 = **+2 195**（预算约 3 000，拆分门槛 3 200）。另 Slint：`ui/palette.slint` 新增 27 行。SDK 与运行器本批 +约 590（`drops.ts` 120、`appearance.ts` 71、`settings.ts` 102、TestHost 约 +150、`types.ts` 约 +110）。

**给协调者的 GUI 实测要点**

1. 配色：探针（或作者 E 的 preferences）`appearance.set("classic")` 后 1 秒内：底栏进度条黄色纯色、菜单选中项、已打开的工具窗口（如音频工作台）、浮动侧栏、选项窗口同时变色；拖着音量滑块时切换滑块不中断；重启后仍为参考播放器；停用调用者后不变；切回 Vestra 与切换前截图一致（渐变恢复）。
2. 设置页：两个探针各注册一页（不同分类）后 F5/`show()`：「选项」640×480、树按九类顺序、空分类不显示；点页面切换；窗口内按 Space 不暂停；⌘W 关闭；再开显示上次页面（重启后也是）；停用其中一个插件 1 秒内该页消失（正在显示则切到同类第一页 / 常规 / 「没有可设置的项目」）；启用后恢复；页面里的输入框与滑块可用，回调到达注册它的插件；页面超高时右侧可滚动。
3. 拖放（物理拖放，**必须实测**）：Finder 拖 1 个 mkv 到视频区 → 提供者收到 1 个路径；同时拖 3 个文件 → 一次调用、顺序与 Finder 一致；拖文件夹 → `folder` 提供者收到目录路径；按住 Option 松手 → `modifiers.alt: true`（Shift、⌘ 同理）；拖到附着侧栏第 3、4 行之间 → `onDrop` 收到的 `at.y` 与面板内行位置一致（与面板里 `mouse-y` 对照）；拖到浮动侧栏窗口 → 同样走 `onDrop`、坐标为窗口内容坐标；拖到底栏 → 底栏插件有监听时收到、否则按扩展名分发；拖 `.srt` 且无提供者 → 无反应；拖到工具窗口 → 无反应；拖放期间播放不暂停。若出现一次拖放被拆成多组或坐标偏一个标题栏高度，按第 6 条降级并告诉维护者。

### 6.3 作者交付

（各作者完成时填写）

### 6.4 验收结果

（P3 填写）
