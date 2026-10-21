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

### 4.1 P0 交付（维护者，2026-10-21）

`src/scripts/check-portability.mjs`（临时，正式 CI 建立后删除），`src/` 下 `npm run portability`（`-- --skip-host` / `-- --skip-scan` 只跑一半）。退出码 0 表示宿主完整通过且扫描没有系统命令。

- **宿主**：`cargo check --locked --target x86_64-pc-windows-msvc --all-targets`（`--target-dir host/target/portability`，不影响平常的构建）。有 rustup 时缺目标自动 `rustup target add`；本机是 Homebrew Rust（没有 rustup），脚本改用工具链自带的 rust-src：`RUSTC_BOOTSTRAP=1 … -Zbuild-std=std,panic_abort`，只检查不链接。都没有时打印安装提示，结果是「NOT CHECKED」而不是通过。依赖的 build script 或 C 代码不能交叉检查时，列出依赖名与原因，再用 `--no-default-features` 检查可检查的部分，结果标「PARTIAL」并以非零退出。另外列出宿主 Windows 下的警告（按文件）和平台模块以外的 `cfg(unix)` / `cfg(target_os)` / `std::os::`（提示，不计失败）。
- **扫描**：用 TypeScript 解析 `src/plugins`、`src/sdk`、`src/runtime` 的 `.ts/.mjs/.js` 等（跳过 `tests`、`dist`、`vendor`、`node_modules`、`*.test.*`），报告字符串字面量里的系统程序路径（`/usr/bin/`、`/usr/sbin/`、`/bin/`、`/opt/homebrew`、`/Library/` 等），`spawn`/`execFile`/`exec` 第一个参数是 macOS 专有命令名（`open`、`sips`、`plutil`、`security`…）的调用，以及 `process.platform` 分支（提示，不计失败）。注释里的命令名不算。

初始结果（`rebuild/portability` 2729deb）：

宿主：
- **交叉检查本身能跑通**：rsmpv（libmpv）、cpal、rfd、slint 的 build script 都不需要目标平台的 C 库，`cargo check` 不链接，所以没有阻塞依赖。真正链接 Windows 可执行文件还需要 MSVC 链接器与 Windows 版 libmpv，本轮不做（§3 末段）。
- 唯一的错误：`src/ui_image.rs:289` 测试里无条件调用 `std::os::unix::fs::symlink`（只在 `--all-targets` 下出现）。
- Windows 下 52 条警告，都是只在 Unix 用到的代码：`media_worker.rs` 17、`native_subtitle.rs` 10、`offline_descriptor.rs` 7、`producer.rs` 7、`producer_audio.rs` 6、`subtitle_backend.rs` 3、`offline_audio.rs` 1、`source_backend.rs` 1。原因是这些功能在 Windows 下是 `cfg(not(unix))` 的「不可用」桩：截图 PNG 导出（`media_worker::encode`，用 Unix 文件描述符把租约文件接到 ffmpeg 标准输入）、生成流传输（`producer.rs`，Unix 域套接字）、本地媒体的 `fd://` 播放（`source_backend.rs`）。
- 平台模块以外的系统相关代码 49 处：`clipboard.rs`、`confirm.rs`、`platform_window.rs`（`cfg(target_os = "macos")` 与「不支持」桩），`main.rs:3434`（双击间隔），`media_worker.rs`、`producer.rs`、`source_backend.rs`、`source_lease.rs`、`package_stage.rs`、`font_carrier.rs`、`ui_image.rs`（`cfg(unix)`、`std::os::unix`）。

扫描（10 处命令，3 处平台分支）：

| 位置 | 内容 |
| --- | --- |
| `sdk/plugin/local.ts:118`、`:125` | `/usr/bin/open`（`system.reveal` / `openPath` / `openUrl`） |
| `plugins/capture/sips.ts:13` | `/usr/bin/sips`（JPEG/BMP 转换） |
| `plugins/info/sysinfo.ts:24–28` | `/usr/bin/sw_vers` ×2、`/usr/sbin/sysctl` ×2、`/usr/sbin/system_profiler`（系统信息） |
| `plugins/playlist/drop.ts:51` | `/usr/bin/plutil`（二进制 `.webloc`） |
| `plugins/subtitle-online/secrets.ts:9` | `/usr/bin/security`（钥匙串） |
| `plugins/subtitle-online/secrets.ts:28` | 平台分支 `process.platform === "darwin"` |
| `plugins/youtube/service.ts:38`、`:41` | 平台分支 `process.platform !== 'win32'`（进程组；已有 Windows 分支，可保留） |

SDK 里另有一个扫描查不出的问题：`local.ts` 的 `locate` 按 `PATH` 拼 `ffmpeg`，Windows 上可执行文件叫 `ffmpeg.exe`，会找不到（P1 一并修）。
