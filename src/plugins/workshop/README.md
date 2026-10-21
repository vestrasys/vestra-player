# 插件工坊（实验性）

> 实验性功能：插件 API 已是正式版 `vestra/1`，工坊本身（生成流程、需求格式、模型接入）仍是实验性的，可能不兼容地改变。

**功能**：在原生窗口中选择专用工作区、输入或选择计数器需求 JSON，点击生成后由 Pi 官方 Agent（`@earendil-works/pi-agent-core` 1.3.0）真实执行工具循环生成候选插件：模型只是本地确定性 mock（`streamFn`），不读凭据、不调用任何网络或付费模型。第一稿故意含类型错误，只有拿到真实 `TS2322` 诊断（随包分发的 TypeScript 5.9.3）后才修复。候选交宿主校验，之后由用户明确点击试载 / 停止试载 / 安装 / 回退上一版本；“取消任务”中止生成、停止试载或取消包操作。

生成的计数器是独立的 SDK v2 插件（默认导出 `activate(plugin)`，`plugin.windows.create("counter.slint")`）：计数从 0 起，“增加”按步长递增，“归零”复位。它只对 `types/`（`vendor/sdk` 的副本）做类型检查，运行时不引用工坊或仓库 SDK。

**入口**：右键菜单顶层「插件工坊…」（`windows` 组 order 820）。窗口为深色主题（`@vestra/theme.slint`、`@vestra/widgets.slint`），没有内部大标题；底部状态只显示一行中文（完整文字在悬停提示里），日志把 Pi 工具事件与 TypeScript 诊断整理成可读的行，不显示修订号、摘要等内部值。源码 `main.ts`（窗口与包生命周期）、`engine.mjs`（Pi 接入、候选文件工具、离线 tsc，类型见 `engine.d.mts`）、`workshop.slint`。

**限制**：只支持 `{"kind":"native-counter","id":"community.counter","version":"1.0.0","step":1}` 与 `1.1.0` / `step:3`；需求最大 64 KiB，最多 3 轮模型、12 次工具动作、总时限 120 秒；日志窗口只保留最后 16 KB。每次生成写入工作区下新的 `candidate-<uuid>/`。“查询试载”按钮保留但恒为禁用：试载首答复丢失由 SDK 按原请求取回。

**开发**（在 `src/` 下）：`npm run plugin -- check|build|test workshop`。构建用 esbuild 把 Pi 依赖打包进 `dist/main.js`（`package.json` 的 `"vestra"` 配置），`vendor/`（TypeScript、Node 类型、SDK 副本）原样复制；第三方许可证见 `licenses/` 与 `dependency-inventory.json`。`vendor/sdk` 必须与 `src/sdk` 一致（不含 `plugin/testing.ts`），`tests/vendor-sdk.test.mjs` 会在不一致时失败；同步命令见该测试文件开头。测试只写系统临时目录。
