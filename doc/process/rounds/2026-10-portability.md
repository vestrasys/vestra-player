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

### 4.2 P1 交付（维护者，2026-10-21）

**宿主平台模块** `src/host/src/platform/`：

- `mod.rs`：公共类型（窗口编号、屏幕几何、`OsFacts`、`Display`）、按 `cfg(target_os)` 选实现、每个实现必须提供的函数清单（模块文档里的表），以及 macOS 与 Windows 共用的 keyring 密钥存储。
- `macos.rs`：原 `platform_window.rs` 的 AppKit 窗口代码（原样搬入），原 `clipboard.rs` / `confirm.rs` 的 NSPasteboard、NSAlert 代码，`caffeinate`（原 `power.rs`），`/usr/bin/open`（原 SDK `local.ts`，10 秒超时与 stderr 末行的错误语义照旧），钥匙串，NSProcessInfo + `sysctl -n hw.model`，NSScreen 显示器，数据目录、Hiragino 字体、默认 FFmpeg 路径、双击间隔、Slint 的 ⌘/⌃ 对调。
- `windows.rs`：窗口经 winit（Slint 的后端）：宿主每次 `show()` 后调用 `platform::shown(window)` 登记（winit 没有「本应用的全部窗口」），置顶为 `WindowLevel::AlwaysOnTop`（之后新开的窗口也置顶），`resize_content` / `screen_for` 用 winit 的内外尺寸与显示器；剪贴板 arboard；确认框 rfd 消息框；`system.*` 用 opener；密钥用 keyring（凭据管理器）；系统信息读注册表（windows-registry）；数据目录 `%APPDATA%\Vestra`；字体微软雅黑；FFmpeg 从 `PATH` 找 `ffmpeg.exe`。
- `other.rs`：Linux 等，全部 `unsupported`（窗口观察为 `Unavailable`），数据目录 `$XDG_DATA_HOME/vestra`。
- 公共代码里原来的 `cfg(target_os)`（`clipboard.rs`、`confirm.rs`、`main.rs` 双击间隔与修饰键、`bindings.rs` 五处）和写死的 `~/Library/Application Support`（`plugins.rs`、`package_manager.rs`、`ui_state.rs`）、Hiragino 路径（`ui.rs`）、`/opt/homebrew/...ffmpeg`（`media_jobs.rs`）都改为调用平台模块。`platform_window.rs` 删除。
- `power.rs` 只保留与系统无关的合并逻辑（`Hold { display }`），启动断言由 `platform::sleep_launcher()` 提供。

**`unsafe` 子包** `src/host/unsafe/win32`（`vestra-win32`，只在 Windows 编译）：`GetCursorPos`（拖放时的指针位置，winit 0.30 不报告拖动位置）、`GetKeyState`（拖放时按住的修饰键）、`GetDoubleClickTime`、`SetThreadExecutionState`（阻止休眠）。理由：这四个调用没有成熟的安全封装 crate（`keepawake` 只为一个调用引入一整个依赖，`device_query` 需要额外权限模型），每个都是一次传值调用或一个局部输出参数，每处 `unsafe` 带 SAFETY 注释，对外 API 安全。其余 Windows 功能都用成熟的安全 crate：arboard 3.7、keyring 3.6（`windows-native`；macOS 用 `apple-native`）、opener 0.9（`reveal`）、windows-registry 0.6，以及已有的 rfd、winit。宿主主包仍是 `forbid(unsafe_code)`，`grep -rnw unsafe src/host/src` 为空。

**新请求**（`src/host/src/system.rs`，参数检查在这里，系统调用在平台模块；可能阻塞的调用在工作线程上执行，结果以 `Event::Reply` 返回）：`system.reveal` / `openPath` / `openUrl`（不需要能力）、`secrets.get` / `set` / `delete`（新能力 `secrets`，服务名为插件 manifest id）、`app.system`（不需要能力）。截图导出新增任务类型 `rawJpeg` / `processedJpeg`（`quality` 1–100，默认 90）、`rawBmp` / `processedBmp`：FFmpeg 照旧写 PNG，校验后由 `image` crate 转为 JPEG 或 24 位 BMP，再不覆盖地发布（`media_worker::convert_still`；`image` 增加 `bmp` 特性，不引入新 crate）。

**SDK**：`system.*` 改为发请求（路径仍在插件进程里转绝对路径、检查存在）；`plugin.secrets`；`plugin.app` 的类型为 `App`（`AppInfo` + `system()`，`system` 不可枚举，旧的 `AppInfo` 字面量和 `deepEqual` 不受影响）；`Snapshot.export` 支持 `jpeg`（`quality`）与 `bmp`（新类型 `ExportOptions`）；`local.ts` 作为 SDK 唯一的平台文件，去掉 `/usr/bin/open`，`tools` 在 Windows 上按 `PATHEXT` 找可执行文件（`executableNames`）。`LocalServices.open` / `openUrl` 改为可选并标为废弃（见 §4.4 兼容期）。`TestHost`：`system.*` 请求经 `host.local.open` / `openUrl`（旧 fake 照常拦截），新增 `host.secrets`、`host.systemInfo`、`host.exports`。构建工具与宿主的能力表加 `secrets`。workshop 的 `vendor/sdk` 已重同步。

**文档**：参考文档 §1 前言（系统与 `unsupported` 原则）、§2 能力表、§3.4 Windows 数据目录、§3.5 `app.system()`、§4.1 Windows 的 Ctrl/Cmd、§6.6 置顶与可见区、§6.7 拖放修饰键、§7 Windows 确认框、§10 JPEG/BMP 与 Windows 导出不可用、§13 `unsupported`、§14 TestHost、§16 各节平台差异、新 §16.6 密钥、新 §16.7 平台差异汇总、§17「可移植性轮 P1」变更说明。

**P0 脚本补充**：Windows 警告按位置去重（之前二进制和测试各报一次）；平台模块以外写死的系统路径（`"/usr/`、`"/opt/`、`"/tmp` 等，跳过注释）也列为提示。

**移植检查现状**（`npm run portability`）：宿主 `OK: the host compiles for Windows`（无阻塞依赖）；扫描剩 8 处命令（全部在插件里，P2 处理）与 4 处 `process.platform`（youtube 两处可保留，subtitle-online 一处 P2 删除，`sdk/plugin/local.ts` 是 SDK 平台文件本身）。退出码在 P2 完成前为 1，`-- --skip-scan` 只看宿主。

**宿主仍未做的 Windows 部分**（不在本轮 P1 清单内，列给协调者决定何时做）：

| 位置 | 现状 | 影响 |
| --- | --- | --- |
| `media_worker.rs` `encode`（`cfg(unix)`） | 把持有的源文件以 `/dev/fd/0` 接到 FFmpeg；Windows 是「不可用」桩 | Windows 上截图与片段导出以 `failed` 结束（参考文档 §10 已写明） |
| `producer.rs`（Unix 域套接字、`/tmp`） | Windows 返回 `unsupported` | Windows 上 `player.openStream`（nes、visualizer 等生成流）不可用 |
| `source_backend.rs`（`fd://`） | Windows 走普通路径打开 | 播放中被替换的本地文件不再受租约保护 |
| `source_lease.rs`、`package_stage.rs`（`dev`/`ino`） | Windows 只比较长度与修改时间 / 总是相同 | 文件身份检查变弱 |
| `font_carrier.rs`（权限位） | Windows 跳过 | 无 |
| rfd 确认框按钮文字 | 未启用 `common-controls-v6`（需要应用清单，否则启动失败） | Windows 显示系统的「确定」「取消」 |
| 运行器路径 `Resources/runtime/runner.ts` | macOS 应用包布局 | Windows 打包时要定布局 |
| `other.rs` | Linux 交叉检查被 `yeslogic-fontconfig-sys` 的 pkg-config 阻塞，未检查 | 无（本轮不做 Linux） |

**检查**（macOS，2026-10-21，均实际执行）：

- `src/`：`npm run check`、`format:check`、`boundaries` 通过；`test:sdk` 97 通过（新增 `tests/sdk/portability.test.ts` 5 项）；`plugins:build` 通过；19 个插件 `npm run plugin -- test` 全部通过（workshop 的 `vendor/sdk` 重同步后 17 项通过）。
- `src/host/`：`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`（另加 `--features pip`）通过；`cargo test --locked` 234 通过（新增平台、`system.rs`、JPEG/BMP 转换、任务类型等测试）。
- `npm run portability -- --skip-scan`：`OK: the host compiles for Windows`，退出码 0；另用同样方式跑了 Windows 目标的 `cargo clippy`，平台模块与 `vestra-win32` 无警告。完整的 `npm run portability` 因插件里剩下的 8 处命令退出码为 1（P2）。
- `grep -rnw unsafe src/host/src`：空。
- 未执行：Windows 真机运行与链接（没有 Windows 环境与 MSVC 链接器）；macOS GUI 真机回归（协调者 P3，§4.5）；Linux 交叉检查（被 fontconfig 的 pkg-config 阻塞）。

### 4.3 能力请求裁决（§4.3 流程；本轮自上而下，请求来自 §3 的任务单，作者未反驳）

| 请求 | 不改核心的做法与代价 | 裁决 | 理由 |
| --- | --- | --- | --- |
| capture：JPEG/BMP 截图（替代 `sips`） | 插件用纯 JS 编码（如 `jpeg-js` + 自写 BMP）：要打包 npm 依赖，先在 JS 里解码 PNG，4K 帧要几百毫秒到一两秒且占用插件进程内存；或者每个系统各调一个命令 | **改写后接受**：不做通用「图片转换」API，只给 `Snapshot.export` 加 `jpeg`（`quality`）和 `bmp` | 导出格式本来就是核心的职责（离线工作进程、校验、不覆盖发布）；宿主已链接 `image` 的 JPEG 编解码，只加 `bmp` 特性；转换在发布前完成，插件不再需要隐藏的临时 PNG 和失败回退。通用转换只有一个用户，不进核心 |
| info：系统信息（替代 `sw_vers`、`sysctl`、`system_profiler`） | Node `os` 能给 CPU 型号（`cpus()[0].model`）、核数、内存，但给不出系统版本（`os.release()` 是 Darwin 内核版本）、构建号、机型，显示器完全没有；读 `/System/Library/CoreServices/SystemVersion.plist` 又是 macOS 专有路径 | **改写后接受**：`plugin.app.system()` 给 `os`、`name`、`version`、`build`、`model`、`arch`、`displays`；CPU 与内存**拒绝**，用 Node `os` | 系统版本、机型需要系统接口（NSProcessInfo、注册表），显示器是宿主独有的窗口系统信息；诊断、问题报告类插件都会用到，属于通用基础能力。CPU、内存 Node 已跨平台提供，不重复 |
| subtitle-online：密钥存储（替代 `security`） | 插件按系统各调命令（macOS `security`，Windows 只能用 PowerShell 或 `cmdkey`，后者读不出密码）；或者只存 0600 文件（Windows 没有等价的权限位） | **接受**：`plugin.secrets`（新能力 `secrets`） | Node 没有系统密钥库接口；任何有账户的插件（在线字幕、bilibili 登录等）都需要，各插件各写一套就是重复造轮子且行为不一致。按插件 id 隔离，macOS 服务名与 subtitle-online 现有钥匙串项一致，用户已存的密钥不丢 |
| playlist：读二进制 `.webloc`（替代 `plutil`） | 插件用纯 JS 解析 bplist：`.webloc` 只有一个 `URL` 字符串，解析 bplist00 的偏移表与 ASCII/UTF-16 字符串对象约 60 行，无依赖 | **拒绝**，给替代方案（见 §4.4 playlist） | 只有一个插件需要，格式简单且稳定，纯 JS 就能跨平台；不属于核心职责 |
| SDK `system.*`（替代 `/usr/bin/open`） | 留在 SDK，按系统分支调用 `open` / `explorer` / `xdg-open`：Windows 的命令行引号规则会让带空格、`&` 的路径出错 | **接受**（任务单 P1 的一部分） | 打开文件和网址要用系统 Shell 接口（`ShellExecute`、`SHOpenFolderAndSelectItems`），放在宿主平台模块里与其它系统代码在一起，SDK 不再有系统分支 |

### 4.4 给作者的迁移清单（P2）

迁移后每个插件跑 `npm run plugin -- check|build|test <名字>`，并在 `src/` 下跑 `npm run portability -- --skip-host`，扫描结果为零。

- **capture**（`sips.ts`、`tasks.ts`、`tests/fakes.ts`、`README.md`）
  - 删除 `sips.ts`。JPEG/BMP 直接导出：`task.snapshot.export({ format: "jpeg", destination, sampleTime, quality: spec.quality ?? 90 })`、`{ format: "bmp", destination, sampleTime }`。
  - 删除 `tasks.ts` 的 `convert()`、隐藏的 `.<名字>.vestra-tmp.png` 暂存（`staging`）和「无法转换为 JPEG，已保存为 PNG」回退：播放器只在转换成功后发布目标文件，失败时任务 `failed`，按 PNG 失败同样处理。
  - `tests/fakes.ts` 的 `watchHost` 不要再替换 `host.local.open`（兼容期后删除），改为 `host.handle("system.openPath", …)` / `host.handle("system.reveal", …)` 注入失败；`fakes.converted` 改为断言 `host.exports` 里的 `kind`（`rawJpeg`、`rawBmp`）与 `quality`。
  - README 删掉 `sips` 的描述。
- **info**（`sysinfo.ts`、`properties.ts`、`dialog.ts`、测试、`README.md`）
  - 删除 `sysinfo.ts` 的三个命令。`const system = await plugin.app.system()`；系统版本行由 `system.name`、`system.version`、`system.build` 组成（如「macOS 26.2（25C56）」，Windows「Windows 11 23H2（22631.4460）」），机型 `system.model`；CPU 用 `os.cpus()[0]?.model` 加核数，内存照旧用 `os`。
  - 显示器行改用 `system.displays`：`name · width x height · (width/scale) x (height/scale) @ refreshRate Hz`；删除 `parseDisplays`（`system_profiler` 的 JSON）。
  - `SystemFacts.macos` 改名为与系统无关的字段（如 `os`），标签「macOS 版本」改为「系统版本」。测试改用 `host.systemInfo` 预置（可再测一组 `os: "windows"`）。
- **playlist**（`drop.ts`）
  - 删除 `execFile("/usr/bin/plutil", …)`。二进制 `.webloc`（以 `bplist00` 开头）用纯 JS 解析：读文件尾 32 字节的 trailer（偏移表项大小、对象引用大小、对象数、顶层对象、偏移表位置），按偏移表找到顶层字典（`0xD?`），在键里找 ASCII 字符串（`0x5?`）`URL`，取对应值的字符串（`0x5?` ASCII 或 `0x6?` UTF-16BE，长度 `0x?F` 时后跟一个整数对象）。XML `.webloc` 的现有正则不变。加一个二进制 `.webloc` 的测试样本（可用 `plutil -convert binary1` 在 macOS 上生成一次后提交为测试数据）。
- **subtitle-online**（`secrets.ts`、`main.ts`、`manifest.json`、测试）
  - manifest 加能力 `"secrets"`。
  - `Secrets` 改为用 `plugin.secrets.get/set/delete`（空值时 `delete`）；删除 `SECURITY`、`execFile`、`process.platform === "darwin"` 分支和 `VESTRA_SUBTITLE_ONLINE_KEYCHAIN`（测试改用 `host.secrets`）。
  - `plugin.secrets` 以 `unsupported` / `unavailable` 失败时保留现有退路：数据目录里的 `secrets.json`（0600）或只在本次运行内记住。
  - `location` 文案：按 `(await plugin.app.system()).os` 显示「macOS 钥匙串」/「Windows 凭据管理器」/「仅本次运行」等。服务名就是 manifest id `vestra.subtitle-online`、账户名是密钥名，和现在写入的钥匙串项相同，用户已存的密钥可直接读到（播放器第一次读取时 macOS 会弹一次授权）。
- **youtube**：`service.ts` 的两处 `process.platform !== 'win32'` 是进程组的跨平台写法，保留，不算问题。
- **playback**：README 里「有视频 `-d -i`、纯音频 `-i`」「caffeinate」的描述改为「播放器持有系统断言（参考文档 §16.2）」。代码不用改。
- **其它插件**：`system.reveal` / `openPath` / `openUrl`、`clipboard`、`dialogs.confirm`、`power`、`drops` 的用法不变，不用改。

**兼容期，P2 全部完成后由维护者删除**：`LocalServices.open` / `openUrl`（`sdk/wire.ts`）以及 `TestHost` 把 `system.*` 转给 `host.local.open` / `openUrl` 的那段（`sdk/plugin/testing.ts`）。目前只有 capture 的 `tests/fakes.ts` 用到。

### 4.5 给协调者的 macOS 回归要点（P3）

macOS 的代码是原样搬进 `platform/macos.rs` 的，但调用路径变了，以下需要真机确认：

1. **剪贴板**：capture「复制到剪贴板」（PNG、JPEG）后在预览里粘贴；info 复制文本。
2. **确认框**：playlist「清空」（带「不再询问」复选框）、plugins「卸载」（带「同时删除数据」）——表单挂在发起的工具窗口上，复选框结果正确；Esc 取消。
3. **置顶**：window 插件「总在最前」后，主窗口、浮动侧栏和之后新开的工具窗口都在其它应用之上；取消后恢复。
4. **拖放**：视频区拖入媒体、字幕（按扩展名分发）；拖到侧栏播放列表面板（`onDrop` 坐标落在正确的行）；按住 ⌥/⇧/⌘ 拖放时 `modifiers` 正确。
5. **视频区尺寸**：`setVideoArea`（window 插件的 1 倍 / 2 倍）保持左上角并留在可见区内。
6. **阻止休眠**：播放视频时 `pmset -g assertions` 有 `caffeinate`（`-d -i -w <pid>`）；纯音频只有 `-i`；暂停后消失。
7. **文件与链接**：capture「打开截图文件夹」、playlist「在 Finder 中显示」（多选同文件夹的几个文件在同一个窗口高亮）、subtitle-online 打开网址——现在由播放器执行 `/usr/bin/open`；不存在的路径仍提示 `missing`。
8. **双击与按键**：`Mouse:LeftClick` 的单击延迟仍按系统双击间隔；`Ctrl+…` 绑定按 ⌘ 也触发；⌘Q 退出。
9. **字体与数据目录**：插件窗口的中文仍是冬青黑体；`~/Library/Application Support/Vestra/` 下的插件数据、安装库、界面状态照常读到（路径未变）。
10. **P2 之后**：capture 的 JPEG/BMP 截图（BMP 与 PNG 逐像素一致，JPEG 质量设置生效，目标已存在时不覆盖）；info 的系统信息页（版本、构建、机型、显示器）；subtitle-online 的密钥（已存的密钥能读到，首次会弹钥匙串授权；保存、清空后在「钥匙串访问」里看到对应变化）。开发版每次重新构建后读钥匙串会再弹一次授权，属预期。

### 4.6 P4 交付（维护者，2026-10-21）

删除 §4.4 的兼容期接口：`LocalServices.open` / `openUrl`（`sdk/wire.ts`，workshop `vendor/sdk` 已重同步）与 `TestHost` 经 `host.local` 转发 `system.*` 的那段（改为直接记录到 `host.system`，注入失败用 `host.handle`），grep 确认插件与测试已无引用，参考 §14/§17 已同步；`src/sdk` 中不再有 `/usr/bin/open`，`npm run portability`（含宿主）clean，src 与宿主的全部检查通过。

### 4.7 协调者验收（2026-10-21）

**全部检查通过**：19 个插件测试、test:sdk 97 个、cargo test 234 个。`npm run portability` 含 Windows 交叉编译检查，结果 clean。

**macOS 真机回归通过**：
- 系统信息页：系统版本、机型、CPU、内存、显示器都正确。
- 截图：核心直接导出 JPEG（40 KB）和 BMP，没有残留的临时文件。
- 「打开截图文件夹」：经宿主的 `system.openPath` 在访达中打开。

**未在本轮验证**：Windows 真机运行。另有几处已知的 Windows 缺口：截图导出、生成流、确认框按钮文字，见 §4.2。
