# 可移植性整改轮（自上而下）

分支：`rebuild/portability`（自 `main` ca548d5）。依据：[社区工作流](../community-workflow.md) §1.5（跨平台规则）、§5（自上而下）。

## 1. 目标

让插件 API 与系统无关，宿主的系统相关代码集中到平台模块，并有 Windows 实现，至少能编译、行为合理；插件不再直接调用 macOS 命令。

不在本轮：GPU 渲染、画中画（原型在 `rebuild/prototypes`，等用户之后裁决）。

## 2. 已知问题清单（协调者 2026-10-21 核对）

**宿主**：
- `power.rs` 无条件启动 `caffeinate`。
- `clipboard.rs`、`confirm.rs` 在非 macOS 系统返回「不支持」。
- `platform_window.rs`、拖放（`drops.rs`）只有 AppKit 实现。
- `bindings.rs` 里 ⌘/⌃ 与 Option 的处理没有 Windows 实测。
- 宿主依赖（`objc2` 等）已经按 `cfg` 分开，但 Windows 下从没编译过。

**SDK 和插件运行器**：`system.reveal`、`openPath`、`openUrl` 写死了 `/usr/bin/open`（`sdk/plugin/local.ts`）。

**插件**：
- capture：`sips`（JPEG/BMP 转换）
- playlist：`plutil`（`.webloc`）
- info：`sw_vers`、`sysctl`、`system_profiler`（系统信息）
- subtitle-online：`security`（钥匙串）

**文档**：需求文档和参考文档里有不少 macOS 专有描述，比如 Finder、钥匙串、⌘ 键。

## 3. 阶段

| 阶段 | 负责 | 内容 | 完成条件 |
| --- | --- | --- | --- |
| P0 | 维护者 | 临时跨平台检查脚本 `src/scripts/check-portability.mjs`（以后写正式 CI 时删除）：<br>· 宿主 `cargo check --target x86_64-pc-windows-msvc`，没装目标或工具链时给出安装提示，并报告哪些依赖阻塞；<br>· 扫描 `src/plugins`、`src/sdk`、`src/runtime` 中的系统命令路径和 macOS 专有调用 | 脚本能跑，列出全部问题 |
| P1 | 维护者 | 宿主平台模块：`src/host/src/platform/{mod,macos,windows,other}.rs`，覆盖阻止休眠、剪贴板、确认框、窗口控制、拖放、打开/显示路径、打开网址。<br>Windows 实现用安全封装，比如 `windows` crate 的安全接口、`arboard`、`rfd`；做不到安全的，按「`unsafe` 只放在 `src/host/unsafe/`」的规则另立子包，并在任务单说明。<br>SDK 的 `system.*` 改由宿主实现，去掉 `/usr/bin/open`。<br>新增核心能力，替换插件里的系统命令（按 §4.3 保守裁决，每项写理由）：<br>· 图片格式转换，或由 `Snapshot.export` 直接输出 JPEG/BMP<br>· 系统信息（`plugin.app` 扩展）<br>· 密钥存储（`plugin.secrets`）<br>· `.webloc` 读取：建议插件自己用纯 JS 解析 plist，不进核心<br>参考文档写明平台差异和 `unsupported` 的情况 | macOS 上全部检查通过；Windows 目标编译通过；移植检查无宿主问题 |
| P2 | 作者 | 按 API 变更迁移插件，删掉直接的系统命令调用，改用新能力或跨平台实现 | 各插件检查、构建、测试通过；移植检查扫描结果为零 |
| P3 | 协调者 | macOS 上真机回归：剪贴板、确认框、置顶、拖放、截图格式、系统信息、钥匙串和密钥存储，然后合并 | — |

Windows 真机运行本轮做不到，只保证编译通过、单元测试覆盖平台无关的逻辑。实机验证以后再说。

## 4. 记录

（各阶段完成后补充）
