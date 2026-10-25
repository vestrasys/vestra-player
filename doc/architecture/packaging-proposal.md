# S7 打包与发布：维护者方案（步骤 7.1）

日期 2026-10-22，基线 main 9027212。只是调研与方案，仓库没有改动。实验都在 scratchpad 里做：release 构建、手工拼一个 `.app`、ad-hoc 签名、无界面启动、生成 DMG、cargo-about 试跑。大文件已经删掉。

结论先行：

- 打包工具**自己写脚本**（Node `.mjs`，放在 `src/scripts/package/`）。不用 cargo-bundle，也不用 cargo-packager。
- macOS 第一版**只出 arm64**，`LSMinimumSystemVersion 14.0`，DMG 用系统自带的 `hdiutil`。
- Windows 用 **Inno Setup**，按用户安装、不需要管理员权限，同时出一个便携 zip。
- libmpv 和 Node 都**固定版本、校验 SHA-256，并镜像到我们自己的发布仓库**。不在 CI 里临时 `brew install`。
- 宿主新增一个平台层函数，统一给出「安装布局」。打包运行时只用自带的 Node，不查 PATH。

有一个许可证问题必须由用户拍板（§5.4）：这个程序链接了 GPL 组件，发布二进制就要同时提供对应源码，这和「源码仓库保持私有」有冲突。

---

## 1. 现状：宿主怎么找各个文件，换成 `.app` 以后哪里会坏

| 东西 | 现在的代码 | 打包后 |
| --- | --- | --- |
| Node | `Command::new("node")`，查 PATH。共 4 处：`plugins.rs:141` 启动插件，`package_manager.rs:171` 校验包时跑 `node --check`，另外 2 处在测试里（`plugins.rs:314` 等）。出错信息是 `start Node plugin runtime (Node >=22.18)` | **坏**。从 Finder 启动时 PATH 只有 `/usr/bin:/bin:/usr/sbin:/sbin`。实测：`env -i PATH=/usr/bin:/bin` 启动 `.app`，18 个内置插件全部 `failed to start` |
| runner | `plugins.rs:125`：`exe/../../Resources/runtime/runner.ts`，找不到就用 `CARGO_MANIFEST_DIR/../runtime/runner.ts`（编译时写死的仓库路径） | macOS 能用。Windows 不能用，因为 `exe.parent().parent()` 是安装目录的上一级 |
| 内置插件 | `package_manager.rs:1185`：`$VESTRA_BUILTIN_PLUGINS`，否则 `exe/../../Resources/plugins`，并且要求那里有 `builtin.json` | macOS 能用（实测 18 个都装进了安装库）。Windows 同上，不能用 |
| UI（`host.slint`、`@vestra`） | `ui.rs:12`：`exe/../../Resources/ui`，否则 `CARGO_MANIFEST_DIR/ui` | macOS 能用。Windows 不能用 |
| 字幕字体 Noto Sans CJK SC | `font_asset.rs:14` 用 `include_bytes!` 编进二进制（16 MB），运行时由 `font_carrier.rs` 写到临时目录 | 能用。只需要把 OFL 许可证（`src/assets/fonts/noto-cjk/LICENSE`）一起发出去 |
| libmpv | 编译时由 rsmpv-sys 按 `MPV_LIBRARY_DIR` 链接。macOS 二进制里写的是绝对路径 `/opt/homebrew/opt/mpv/lib/libmpv.2.dylib`。Windows 的 `libmpv-2.dll` 放在 exe 旁边 | macOS 不处理就**坏**：没装 brew 的机器上直接 dyld 失败。要把 50 个 dylib 收进 `Frameworks/` 并改成 `@rpath`（实测可行，见 §3.3） |
| FFmpeg 命令行（离线导出、内嵌字幕提取） | `media_jobs.rs:604`：`$VESTRA_FFMPEG`，否则 `platform::default_ffmpeg()`。macOS 上这个默认值写死成 `/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg`，Windows 上去 PATH 找 `ffmpeg.exe`。ffprobe 取 ffmpeg 同目录下的那个（`media_worker.rs:548`） | **只在开发机上能用**。用户机器上截图导出、片段导出、内嵌字幕都会是 `unavailable` |
| 插件里的 ffmpeg/ffprobe | SDK `plugin.tools.run`（`dist/plugin/local.js:102`）：`VESTRA_FFMPEG`/`VESTRA_FFPROBE`，否则查 PATH | 同上。会影响 playlist 的时长、info、缩略图、visualizer、capture，菜单会显示「需要 ffmpeg」 |
| workshop 插件启动子进程 | `engine.mjs:84` 用 `spawn(process.execPath, …)` | 不用改，自动用到自带的 Node |
| 数据目录 | `platform::app_data_dir()`：macOS 是 `~/Library/Application Support/Vestra`，Windows 是 `%APPDATA%\Vestra`。下面有 `packages/`、`plugin-data/`、`ui-state.json` | 不受影响。实测安装库和插件数据都写到了这里 |
| 日志 | 插件的 stderr 直接继承宿主的 stderr | 从 Finder 双击启动时 stderr 被丢掉。Windows 改成 GUI 子系统以后也没有 stderr |
| 命令行参数 | `main.rs:3998`：不认识的参数直接 `bail!` | 现在没问题。但以后做文件关联时，系统传进来的文件路径会被这里拒绝（见 §3.2）。2026-10-25：位置参数改为要打开的文件（`launch.rs`） |
| Windows 控制台 | 没有 `#![windows_subsystem = "windows"]`，启动子进程也没加 `CREATE_NO_WINDOW` | 双击 exe 会多出一个控制台窗口，每个 Node 和 ffmpeg 子进程还会再各开一个 |

只在开发时有意义的东西：`CARGO_MANIFEST_DIR` 的三个回退；`default_ffmpeg` 写死的 Homebrew 路径；测试里直接 `Command::new("node")`；`VESTRA_*` 环境变量（这些继续保留，用来调试）。

**实验证据**：把 release 二进制、Homebrew 的 libmpv 闭包、Node 24.22.1 和 `stage-runtime`/`stage-plugins` 的输出拼成一个 `.app`，做 ad-hoc 签名。

- 用 `PATH=/usr/bin:/bin` 启动：18 个插件全部失败。
- 只把 `Contents/MacOS`（里面放着 node）加进 PATH 再启动：15 个插件 `runtime.ready`。另外 3 个是 `mounts require a native shell`，这是无界面模式下面板插件本来就会报的。
- `DYLD_PRINT_LIBRARIES` 显示：从 `Contents/Frameworks` 加载了 50 个库，`/opt/homebrew` 一个也没有。
- 结论：打包布局本身没问题，缺的只是宿主去哪里找 Node。

## 2. 内置 Node

### 2.1 用哪个版本（要协调者决定）

nodejs.org 的 `schedule.json` 和 `dist/index.json`，2026-10-22 查询：

| | 当前补丁版本 | Active LTS | 进入维护期 | 结束支持 |
| --- | --- | --- | --- | --- |
| 24 Krypton | **24.22.1**（2026-10-20，安全更新） | 2025-10-28 | **2026-10-20（已经进入）** | 2028-04-30 |
| 26 | 26.12.1（2026-10-21，安全更新） | **2026-10-28（6 天后）** | 2027-10-20 | 2029-04-30 |

- 生态计划写的是 Node 24。但现在 CI（`setup-node` 用 `node-version: "26"`）和本机（v26.12.0）都在跑 26。也就是说，**所有测试实际上是在 26 上通过的，24 一次也没测过**。
- 我的建议是改成 **Node 26 LTS**，理由有三：
  - 开发环境、CI 和最终产品用同一个版本；
  - 支持期长一年；
  - `vestra/1` 期间 Node 主版本只升不降，从 24 起步的话 2028-04 之前必须升一次。
- 如果维持 24：CI 的 `setup-node` 要同时改成 24，保证测的就是发出去的那个版本。
- 不管选哪个，规则都一样：CI、打包、SDK 类型三处用同一个主版本。补丁版本随播放器的小版本更新。
- 以下用 `<NODE>` 表示选定的版本，例子按 24.22.1 写。

### 2.2 从哪里下载、怎么校验

- 来源是官方 `https://nodejs.org/dist/v<NODE>/`：
  - `node-v<NODE>-darwin-arm64.tar.xz`，24.22.1 是 27.5 MB，26.12.1 是 30.7 MB；
  - `node-v<NODE>-darwin-x64.tar.xz`，29.3 MB；
  - `node-v<NODE>-win-x64.zip`，37.7 MB；
  - 另外有 `win-arm64`，留到以后。
- 校验分两层：
  1. 新增 `src/packaging/node.json`，写 `{ version, sha256: { "darwin-arm64": …, "win-x64": … } }`。CI 下载以后只比对这个 SHA-256，和 `setup-libmpv` 里 mpv-dev 的做法一样。
  2. 升级 Node 用脚本 `src/scripts/package/update-node.mjs <版本>`：下载 `SHASUMS256.txt` 和 `SHASUMS256.txt.sig`，用 `gpgv --keyring pubring.kbx` 校验（keyring 取 `nodejs/release-keys` 仓库固定提交的 `gpg-only-active-keys/pubring.kbx`），通过以后把哈希写进 `node.json`。CI 打包时也重复这一步，开销很小，作为第二道保险。
  - 实测：24.22.1 的签名 `Good signature from "Antoine du Hamel"`，`darwin-arm64.tar.xz` 的 SHA-256 与 SHASUMS 一致。
- 包里**只放 `node` 可执行文件和它的 `LICENSE`**。不放 npm、npx、corepack、`include/`（64 MB）、`share/`。
- 体积：arm64 的 `node` 解压后 122 MB（官方已经 strip 过），只依赖系统框架和 `libc++`，`minos 13.5`。

### 2.3 架构与位置

- macOS 第一版**只做 arm64**。
  - x64 以后单独出一个 DMG，不做 universal：universal 要把每个 dylib 和 Node 都 `lipo` 成双架构，体积接近翻倍，而 Homebrew 在一台 runner 上只能装出一种架构。
  - Node 的 macOS x64 从 26 开始降为 Tier 2，支持到 2028 年初。是否出 x64 要协调者决定。
- Windows 第一版只做 x64，arm64 以后再说（shinchiro 有 aarch64 的 mpv-dev，GitHub 也有 `windows-11-arm` runner）。
- 文件位置：
  - macOS：`Vestra Player.app/Contents/MacOS/node`。苹果的规定是可执行的辅助程序放在 `MacOS/` 或 `Helpers/`，放在 `Resources/` 签名和公证时会出问题；
  - Windows：`<安装目录>\node\node.exe`。

### 2.4 宿主怎么解析：平台层

新增 `platform::install_layout(exe: &Path) -> Option<InstallLayout>`。每个系统一份实现，都是纯函数，任何机器上都能测：

```text
InstallLayout { resources, node, ffmpeg: Option, ffprobe: Option }
macOS:   exe = …/X.app/Contents/MacOS/vestra-host，并且 Contents/Info.plist 存在
         → resources = Contents/Resources，node = Contents/MacOS/node，ffmpeg/ffprobe = Contents/MacOS/ff*
Windows: exe 所在目录下有 resources\runtime\runner.ts
         → resources = <dir>\resources，node = <dir>\node\node.exe，ffmpeg = <dir>\ffmpeg\ff*.exe
Linux:   以后再定（AppImage 或 /usr/lib/vestra）
```

公共代码里加一个 `runtime_paths.rs`，启动时只解析一次，得到 `RuntimePaths { node, runner, ui, builtin_plugins, ffmpeg, ffprobe, packaged: bool }`。

- **打包运行**（`install_layout` 有结果）：
  - Node 依次取 `$VESTRA_NODE`（显式调试用）、自带的 node。**不查 PATH**，否则用户机器上随便哪个版本的 Node 都可能被用上，`vestra/1` 对 Node 版本的承诺就不成立了。
  - 自带的 node 不存在时，启动阶段就报一个明确的错（有界面时弹确认框，见 `platform::confirm`）。不要等到 18 个插件逐个失败。
- **开发运行**：Node 依次取 `$VESTRA_NODE`、PATH 上的 `node`，和现在一样。runner、UI 用 `CARGO_MANIFEST_DIR`。
- 4 处 `Command::new("node")`、`ui_dir()`、`builtin directory()`、`default_ffmpeg()` 全部改成从 `RuntimePaths` 取。
- 打包运行时启动插件进程：
  - 去掉 `NODE_OPTIONS` 和 `NODE_PATH`，免得用户环境里的 `--require` 之类混进插件进程。`NODE_EXTRA_CA_CERTS` 保留，企业代理需要它；
  - 如果用户没有设置 `VESTRA_FFMPEG`/`VESTRA_FFPROBE`，就把它们设成自带的那两个。SDK 本来就读这两个变量，**SDK 不用改**。

### 2.5 Node 启动参数

不加任何参数。

- 24.22.1 和 26 对 `runner.ts` 做类型剥离时都不再输出 ExperimentalWarning（实测 24.22.1 的 stderr 里没有 warning），所以用不着 `--disable-warning`。
- npm 和 corepack 不放进包里，等于已经禁用了。
- 不加 `--no-addons`：插件是受信任的完整 Node 进程，生态计划 1.6 第 4 条已经这么定了。

### 2.6 SDK

- 现在 SDK 1.1.1 写的是 `engines.node: ">=22.18"`、`dependencies["@types/node"]: ">=22.18"`。后一个会解析到最新的 `@types/node`（25、26），插件作者因此能拿到比播放器实际 Node 更新的 API 类型，这是个实际的坑。
- 改成：`engines.node: ">=<MAJOR>"`；`@types/node: "^<MAJOR>.0.0"`，也就是首个内置主版本的类型。主版本只升不降，所以这个下限一直成立。
- 播放器这边：`src/package.json` 的 devDependency `@types/node ^25` 改成同一个主版本。文档里的 Node ≥ 22.18（`reference.md:8`、`tutorial.md:317`、`store-design.md:203`、架构 README §2）改成「播放器内置 Node <MAJOR>」。
- **要发一个 SDK 版本，但不阻塞打包**。建议发 1.2.0：依赖范围收紧算小版本，不改 `apiVersion`。和 S7 合并一起发。

## 3. macOS

### 3.1 包的布局

```text
Vestra Player.app/Contents/
  Info.plist
  MacOS/vestra-host          主程序（CFBundleExecutable）
  MacOS/node                 Node
  MacOS/ffmpeg, MacOS/ffprobe  与 libmpv 共用同一套 libav*（多出约 0.7 MB）
  Frameworks/*.dylib         libmpv 闭包，install name 都是 @rpath/<名字>
  Resources/AppIcon.icns
  Resources/ui/              src/host/ui
  Resources/runtime/         stage-runtime.mjs 的输出（另外补一份 SDK 的 LICENSE）
  Resources/plugins/         stage-plugins.mjs 的输出（18 个 .vxp 和 builtin.json）
  Resources/licenses/        见 §5
```

实测体积：

| 部分 | 大小 |
| --- | --- |
| 主程序 | 47 MB（strip 以后 41.5 MB，建议加 `[profile.release] strip = "symbols"`） |
| Frameworks | 61 MB |
| Node | 122 MB |
| plugins | 4.4 MB |
| 整个 `.app` | 231 MB |
| DMG（ULMO/lzma） | **76 MB**（UDZO 是 107 MB） |

### 3.2 Info.plist（提议）

| 键 | 值 |
| --- | --- |
| `CFBundleIdentifier` | **定为 `com.vestraplayer.player`**（协调者 2026-10-22 最终决定，取代下面的提议和轮次文档决定 15 的旧值；宿主代码不使用 bundle id：数据目录是 `Vestra`，钥匙串服务名是 `vestra.plugin.<id>`）。原提议：`com.vestrasys.player`。需要确认 vestrasys.com 这个域名是不是我们的；不是的话用 `io.github.vestrasys.player`。**一旦签名发布就不能再改**（钥匙串、TCC 权限、偏好都挂在它上面） |
| `CFBundleName`、`CFBundleDisplayName` | `Vestra Player` |
| `CFBundleExecutable` | `vestra-host` |
| `CFBundleIconFile` | `AppIcon` |
| `CFBundlePackageType` | `APPL` |
| `CFBundleShortVersionString` | 取 `src/host/Cargo.toml` 的 version |
| `CFBundleVersion` | 同一个值 |
| `LSMinimumSystemVersion` | `14.0` |
| `LSApplicationCategoryType` | `public.app-category.video` |
| `NSHighResolutionCapable` | `true` |
| `NSSupportsAutomaticGraphicsSwitching` | `true`。我们用 OpenGL，不加这一项的话，双显卡机器会被强制切到独显 |
| `NSHumanReadableCopyright` | `© 2026 Vestra Player contributors · GPL-3.0-or-later` |

- **最低系统版本取依赖里最高的那个**：
  - Node 要求 13.5；
  - Rust 的默认部署目标是 11.0；
  - 画中画在运行时判断（`unsafe/pip` 里调用 `isOperatingSystemAtLeastVersion`，菜单只在支持时出现），不影响最低版本；
  - **真正卡住的是 Homebrew bottle**：本机 bottle 的 `minos` 是 26.0。bottle 在哪个系统上构建，最低版本就是那个系统，`macos-15` runner 装出来的是 15.0。
  - 所以 libmpv 的依赖包要在 `macos-14` runner 上生成，见 §6。这样最低是 14.0，与画中画的要求一致。
- **文档类型**：第一版**不声明**，Windows 也不做文件关联。原因是宿主还接不住：
  - macOS 打开文件走 Apple Event（`application:openURLs:`），winit/Slint 没有转发，要在平台层接；
  - 命令行遇到不认识的参数会 `bail!`。
  - 拖放已经能用。接好以后再加 `CFBundleDocumentTypes`（`LSHandlerRank = Alternate`，常见视频和音频的 UTI），作为单独一项。
  - **2026-10-25 已接上**：
    - 命令行的位置参数是要打开的文件（`launch.rs`，`file://` 转成路径，`--` 之后全当文件），打开方式和把同样的文件拖到视频区一样（`open_files.rs` → `drops.drop`）；启动时等已安装插件激活完再送；
    - macOS 在平台层接 open-documents Apple Event（`unsafe/open`，用 `NSAppleEventManager`，winit 的 delegate 不动）；`Info.plist` 声明 `public.movie`、`public.audio`，系统没声明的类型（mkv、mka、ogv 等）用 `UTImportedTypeDeclarations` 补，角色 Viewer、`LSHandlerRank = Alternate`，不抢默认；
    - 单实例（`platform/instance.rs`，各系统同一份代码）：第一个进程锁住数据目录里的 `instance.lock`，在回环端口上监听；后来的进程把文件交过去后退出，正在运行的窗口回到前台（Windows 先 `AllowSetForegroundWindow`）。`--plugin`、`--headless` 的运行不参与，除非设了 `VESTRA_INSTANCE_DIR`；
    - Windows 安装包始终注册 ProgID、`OpenWithProgids`、`Applications\vestra-host.exe\SupportedTypes`（「打开方式」里有 Vestra Player）；可选任务「associate」（默认不勾）再写 `Capabilities` 和 `RegisteredApplications`，用户在「设置 › 默认应用」里自己选。清单在 `packaging/file-types.mjs`，生成 `windows/file-types.iss`。
- **图标**：来源是 `src/assets/brand/logo.png`（1024×1024，带透明），是一个满幅的 V 字，没有底板。
  - 用 `sips` 和 `iconutil` 生成 `.icns` 已经实测可行（806 KB）。
  - 但 macOS 11 以后的图标规范是 824 px 的圆角方形底板加内容；macOS 26 会把不合规范的图标塞进一个灰色底板里。
  - 方案 A（推荐）：打包脚本合成一个深色圆角底板，V 缩到 70% 左右放在中间。
  - 方案 B：先直接用，等设计稿。
  - Windows 的 `.ico` 直接用这个 V 就合适。

### 3.3 libmpv 依赖闭包

- 实测 Homebrew mpv 0.41.0_10 的闭包有 **50 个 dylib，64 MB**，包括 ffmpeg 9.0.2、libass、libplacebo、luajit、mujs、x264、x265、svt-av1、shaderc、vulkan-loader、openssl@4 等。
- 做法（实验脚本大约 40 行，第一次就能跑通）：
  1. 从 `vestra-host` 开始，用 `otool -L` 做广度优先，除 `/usr/lib` 和 `/System` 以外全部拷进 `Frameworks/`；`@rpath`/`@loader_path` 按引用方原来所在的目录解析。
  2. 每个 dylib 执行 `install_name_tool -id @rpath/<名字>`，每个引用执行 `-change <旧> @rpath/<名字>`，再加上 rpath `@loader_path`。主程序加 rpath `@executable_path/../Frameworks`。
  3. `install_name_tool` 会让原有签名失效，arm64 上没有签名的代码不能运行，所以最后**必须重新签名**：没有证书时用 ad-hoc（`codesign -s -`），由内到外：dylib、ffmpeg/ffprobe、node（带 entitlements）、主程序，最后签整个 `.app`。实测 `codesign --verify --deep --strict` 通过。
- 这一步放在「生成依赖包」那个 workflow 里只做一次（§6）。打包时拿到的就是已经改好 `@rpath` 的目录，只需要拷贝和签名。
- **不用 dylibbundler**：多一个 brew 依赖，而且它不处理我们要同时收进来的 ffmpeg/ffprobe。

### 3.4 Hardened Runtime 与 entitlements（为以后签名准备，文件现在就写好）

- `src/packaging/macos/node.entitlements`：`com.apple.security.cs.allow-jit`、`com.apple.security.cs.allow-unsigned-executable-memory`。V8 需要这两项。官方 node 带的另外三项不要：`disable-executable-page-protection`、`disable-library-validation`（只有原生 addon 用得到）、`get-task-allow`（公证不接受）。
- `src/packaging/macos/host.entitlements`：先为空。所有 dylib 都用我们自己的 Team ID 签名，所以不需要 `disable-library-validation`。
  - 风险：libmpv 带了 LuaJIT。宿主没有关掉 mpv 的内置 Lua 脚本（ytdl_hook、stats、console 等，默认是加载的），在 Hardened Runtime 下 LuaJIT 申请不到 MAP_JIT，可能退回解释执行，也可能报错。
  - 建议宿主显式设置 `ytdl=no`、`load-stats-overlay=no`、`load-console=no`、`load-auto-profiles=no`、`load-select=no`、`load-positioning=no`、`load-commands=no`、`load-scripts=no`。网络来源由插件解析，宿主不需要 mpv 自己去调 yt-dlp。这一条要协调者确认：它会改变行为，以前打开 URL 时 mpv 可能会自己调 yt-dlp。
  - 如果实测还需要 JIT，再给 host 加 `allow-jit`。
- 不签名的测试包：到 macOS 15 及以后，从浏览器下载的未签名 app，右键「打开」已经绕不过去了，要到「系统设置 → 隐私与安全性 → 仍要打开」，或者执行 `xattr -dr com.apple.quarantine`。用 `gh run download` 下载的文件不带隔离属性。

### 3.5 DMG

- `hdiutil create -volname "Vestra Player" -srcfolder <目录：.app 加上 /Applications 的符号链接> -format ULMO`，不需要任何额外依赖。ULMO 从 10.15 起支持，实测 76 MB。
- 不做背景图和窗口布局：那要用 AppleScript 控制 Finder，CI 上不稳定。
- 签名以后对 DMG 本身也要 `codesign` 和公证、staple。

### 3.6 工具选择

| 工具 | 结论 |
| --- | --- |
| **自己写脚本**（`src/scripts/package/macos.mjs`，用 `install_name_tool`/`codesign`/`hdiutil`/`iconutil`，都是系统自带） | **推荐**。理由：libmpv 闭包、Node、ffmpeg 的位置都要完全受我们控制；`stage-runtime`/`stage-plugins` 已经是 mjs，能直接复用；Windows 用同一套暂存逻辑；没有新的 Rust 构建依赖，也不用过 cargo-deny |
| cargo-bundle | 不推荐：维护不活跃，不处理 dylib 闭包，也不支持 Node 这种额外的可执行文件 |
| cargo-packager | 不推荐：能出 `.app`/DMG/NSIS/WiX，但闭包照样得我们自己算好喂给它；DMG 依赖 create-dmg 加 AppleScript；在它的约定和我们的布局之间还要多一层适配 |

## 4. Windows

### 4.1 安装包技术

| | Inno Setup | NSIS | WiX（MSI） | MSIX |
| --- | --- | --- | --- | --- |
| 按用户安装，不要管理员 | `PrivilegesRequired=lowest`，装到 `%LOCALAPPDATA%\Programs\Vestra Player`，很简单 | 可以，要手写 | 按用户安装的 MSI 很别扭（`ALLUSERS=2`/`MSIINSTALLPERUSER`） | 天然按用户 |
| 不签名能不能装 | 能（会有 SmartScreen 警告） | 能 | 能 | **不能**：侧载也必须有受信任的签名 |
| 签名 | `SignTool=` 指令，安装程序和卸载程序都签，可以接 Azure Trusted Signing | 可以 | 可以 | 必须 |
| 升级、以后自动更新 | 同一个 AppId 静默覆盖安装（`/VERYSILENT`），重启管理器负责关掉正在运行的程序 | 可以 | 主版本升级规则复杂 | 商店或 App Installer |
| 卸载时清理数据 | `[UninstallDelete]` 加 `[Code]` 询问 | 手写 | 自定义动作 | 自动删虚拟化的 AppData（会把用户数据一起删掉） |
| 文件关联 | `[Registry]` 写 HKCU\Software\Classes | 手写 | 有 | 清单声明 |
| CI 上有没有 | windows-2022 镜像预装 **InnoSetup 6.7.1** | 预装 3.10 | 预装 3.14 | 有 SDK |
| 其它 | 脚本语言是 Pascal | 脚本偏底层 | 适合企业用组策略批量部署 | 文件系统和注册表虚拟化会影响子进程和数据目录 |

**推荐 Inno Setup**，配置文件放在 `src/packaging/windows/vestra.iss`。

- `PrivilegesRequired=lowest`，`PrivilegesRequiredOverridesAllowed=dialog`（需要的人可以选为所有用户安装）。
- `ArchitecturesAllowed=x64compatible`，`MinVersion=10.0`。
- 在开始菜单建「Vestra Player」快捷方式。
- 卸载时问一句「同时删除插件和设置吗？」，默认不删。范围是 `%APPDATA%\Vestra` 和凭据管理器里的条目；后者由宿主的 `secret_index` 记录名字，卸载程序删不干净，可以先不管。
- 同时出一个**便携 zip**，几乎零成本，第一版先有这个。

### 4.2 内容与运行库

```text
<安装目录>\vestra-host.exe   带 VERSIONINFO（FileDescription = Vestra Player）和 .ico
<安装目录>\libmpv-2.dll      shinchiro 版本：MinGW 编译，FFmpeg 静态链接在 DLL 内部
<安装目录>\node\node.exe     官方构建，静态 CRT
<安装目录>\ffmpeg\ffmpeg.exe, ffprobe.exe   见下
<安装目录>\resources\{ui,runtime,plugins}
<安装目录>\licenses\…
```

- **VC 运行库**：
  - `libmpv-2.dll` 只导入系统 DLL 和 `api-ms-win-crt-*`，也就是 UCRT，Windows 10 起系统自带（已经用 strings 检查过）；
  - node.exe 是静态 CRT；
  - 但 Rust 的 MSVC 目标默认动态链接 `VCRUNTIME140.dll`，而 Windows 10/11 不保证有这个文件。
  - 建议在 `src/host/.cargo/config.toml` 里给 `x86_64-pc-windows-msvc` 加 `-C target-feature=+crt-static`，所有构建统一，不用随包带 vc_redist。
- **宿主要改的地方**：
  - release 构建加 `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`；
  - 启动 Node、ffmpeg 等子进程时调用平台函数 `platform::hide_console(&mut Command)`：Windows 上用 std 的 `CommandExt::creation_flags(CREATE_NO_WINDOW)`，不需要 unsafe；其它系统什么都不做；
  - stderr 改写到日志文件（§9）。
- **ffmpeg/ffprobe**：shinchiro 的 ffmpeg 包里只有 `ffmpeg.exe`（103 MB），**没有 ffprobe**。
  - 可以改用 BtbN FFmpeg-Builds 的 `win64-gpl-shared`（带 ffmpeg、ffprobe 和 av*.dll，压缩后大约 35 MB），同样固定版本并镜像。
  - 也可以第一版 Windows 不带，相关菜单显示「需要 ffmpeg」。
  - 要协调者决定。我建议带：macOS 上几乎不增加体积；Windows 也和 macOS 功能一致。
- `libmpv-2.dll` 有 121 MB，因为 shinchiro 的 dev 包是用 `-Ddebug=true` 编译的。以后可以试 `llvm-strip`，第一版不处理。
- 估计安装包压缩后 90–120 MB。

### 4.3 CI 冒烟测试（Windows 真机验收随 S6）

1. `setup.exe /VERYSILENT /CURRENTUSER /DIR=<临时目录>`；
2. 在去掉 node 的 PATH 下运行 `vestra-host.exe --headless --silent --seconds 6`，要求 18 个插件都到 ready，或者是已知的无界面错误；
3. 静默卸载，确认安装目录已经删掉。

## 5. 第三方许可证

### 5.1 Rust：cargo-about

在 scratchpad 里试跑过 cargo-about 0.9.2：

- `--target aarch64-apple-darwin` 和 `--target x86_64-pc-windows-msvc` 都是 `--fail` 退出 0，没有无法识别的许可证。
- Slint 按 `GPL-3.0-only` 解析，宿主自己的 crate 是 `GPL-3.0-or-later`。

配置写在 `src/host/about.toml`：

```toml
accepted = [ …与 deny.toml [licenses].allow 完全相同… ]
targets = ["aarch64-apple-darwin", "x86_64-pc-windows-msvc"]
ignore-build-dependencies = true
ignore-dev-dependencies = true
```

- 注意：`filter-noassertion = true` 在 0.9 里会报配置错误，不要加。
- `accepted` 与 deny.toml 的一致性由一个小脚本检查（`check-licenses.mjs` 的一部分）。两份清单任何一边改了而另一边没改，CI 就失败。
- 输出：在各平台的打包 job 里用 `--target <本平台>` 生成 `licenses/rust-crates.html`（模板 `src/host/about.hbs`，基于 cargo-about 官方的 HTML 示例）。
- cargo-about 在 CI 里用 `taiki-e/install-action` 安装，固定 SHA。

### 5.2 不是 Cargo 管理的组件：清单

新增 `src/packaging/third-party.toml`，每个组件一条：`name`、`version`、`spdx`、`files`（包内路径的 glob）、`license_files`（包内路径）、`source`（源码 URL 和 SHA-256）。

| 组件 | 许可证 | 证据和备注 |
| --- | --- | --- |
| Vestra Player 自身 | GPL-3.0-or-later | **仓库里没有 LICENSE 文件**，要补一份 GPL-3.0 全文，放到包里的 `licenses/LICENSE` |
| libmpv（macOS 用 Homebrew，Windows 用 shinchiro） | GPL-2.0-or-later，因为构建时开了 GPL 选项，组合起来按 GPLv3 | shinchiro 的 `mpv.cmake` 没有关掉 GPL |
| FFmpeg 9.0.2（Homebrew）/ shinchiro 的 FFmpeg | **GPL-3.0-or-later**：`--enable-gpl --enable-version3`，**没有 `--enable-nonfree`，没有 fdk-aac** | `ffmpeg -buildconf`；shinchiro 的 `packages/ffmpeg.cmake` |
| x264、x265、rubberband | GPL-2.0-or-later | 在 Homebrew 闭包里，shinchiro 是静态链接 |
| lame、libbluray 等 | LGPL-2.1-or-later | |
| OpenSSL 4、shaderc、vulkan-loader 等 | Apache-2.0 | 与 GPLv3 兼容 |
| libass、harfbuzz、freetype、fribidi、dav1d、zimg 等 | ISC、MIT、FTL/GPL-2.0、LGPL、BSD | 全部都要列出来 |
| Node.js | MIT。它的 LICENSE 文件里已经包含了它内置依赖（V8、OpenSSL、ICU……）的许可证 | 原样放进包 |
| Noto Sans CJK SC | OFL-1.1 | `src/assets/fonts/noto-cjk/LICENSE` |
| `@vestra-player/sdk`（在 `Resources/runtime`） | Apache-2.0 | `stage-runtime.mjs` 目前**没有拷 LICENSE**，要补 |
| 插件 workshop 打包进去的依赖 | MIT、Apache-2.0 | 已经有 `licenses/` 和 `dependency-inventory.json`，打进 `.vxp` 的 `dist/licenses` |
| 插件 nes 打包进去的 jsnes 2.1.0 | **Apache-2.0** | **`dist/main.js` 里没有许可证文本，也没有 `licenses/`**，违反 Apache-2.0 §4。由插件作者补上 `licenses/jsnes/LICENSE`（同时把 `vestra.assets` 指过去），这是插件改动 |

Homebrew 那部分的清单由依赖包 workflow（§6）根据 `brew info --json=v2` 生成，写成 `deps.json`，包含配方、版本、`license` 字段和源码 URL。shinchiro 那部分按它的 `packages/*.cmake` 手写，只要列出 GPL 和 LGPL 组件。

### 5.3 在包里放在哪里，CI 怎么检查

- 位置：macOS `Contents/Resources/licenses/`，Windows `<安装目录>\licenses\`：
  - `index.html`：总目录；
  - `LICENSE`：Vestra 的 GPL 全文；
  - `rust-crates.html`；
  - `third-party/<组件>/…`。
- 以后在 info 插件的「关于」窗口加一个「第三方许可证」链接，用 `open_path` 打开。
- `src/scripts/package/check-licenses.mjs` 在暂存好的包上检查：
  1. 包里每个文件都要被「自己的文件」或者某条清单的 `files` 覆盖，不能有不认识的文件；
  2. 每条清单的 `license_files` 都确实在包里；
  3. 版本对得上：`node --version` 等于 `node.json`，dylib 和 dll 与 `deps.json` 一致；
  4. 每个 `.vxp`：如果它的 `package.json` 有 `dependencies`，`dist` 里就必须有 `licenses/` 目录。按这条规则，现在的 nes 会失败；
  5. about.toml 和 deny.toml 的许可证列表一致。
- release、nightly、PR 的打包 job 都跑这个检查。

### 5.4 GPL 风险（**要用户决定**）

- 没有发现与 GPLv3 不兼容的组件：没有 nonfree、没有 fdk-aac；OpenSSL 3 以后是 Apache-2.0。
- **但发布的是 GPL 组合作品的二进制**。Slint 是按 GPL-3.0-only 选项使用的；FFmpeg/mpv 用了 GPL 构建，而宿主正依赖其中的 GPL 部分，比如 `hqdn3d` 滤镜只有 GPL 构建才有。按 GPLv3 §6，二进制的接收者必须能拿到**完整的对应源码，包括宿主源码**。这与「播放器源码仓库保持私有」冲突。可选的做法：
  - **(a) 推荐**：每个 release 在公开发布仓库附上 `vestra-player-<版本>-src.tar.gz`（标签处的 `git archive`），再附上第三方源码包（Homebrew 配方的源码 tarball，shinchiro 构建对应的 mpv、FFmpeg 和各个库的提交）。源码仓库本身可以继续私有，但源码对二进制接收者实际上是公开的。
  - (b) 改用 Slint 的商业或免版税许可，再把 mpv 和 FFmpeg 换成 LGPL 构建。代价是失去 `hqdn3d` 等 GPL 滤镜、x264 编码（`media_worker.rs:776` 在用），而且和 Cargo 里已经声明的 GPL-3.0-or-later 矛盾。不推荐。
- 我不是律师，最终由用户决定。

## 6. 输入可复现

- 现在的情况：
  - macOS 在 CI 里 `brew install mpv`，每次装到的是当天的 bottle，版本不固定；最低系统版本跟着 runner 走；只能装出 arm64；
  - Windows 固定了 shinchiro 20261021 那个构建，但 **shinchiro 只保留最近 30 个 release**（现在最早的是 20260919），这个固定的 URL 大约一个月后就会 404。
- 备选方案：

| 方案 | 评价 |
| --- | --- |
| A. CI 里每次 `brew install` | 不可复现，最低系统版本跟 runner 走，排除 |
| B. 自己编译 mpv、FFmpeg 和几十个依赖 | 能完全掌控，但维护成本高，编译要几个小时，以后再考虑 |
| C. 用别人编好的：media-kit/libmpv-darwin-build | 下载检查过 v0.7.3：mpv **0.36.0**（2023 年）、FFmpeg 6、**LGPL 构建，没有 hqdn3d**、没有 libplacebo。太旧，滤镜也不够，排除 |
| C'. MPVKit（xcframework，FFmpeg 9.0.2，有 GPL 版） | 面向 iOS/SwiftPM，拿来给宿主链接要做转换。备选 |
| C''. IINA 的依赖 | 从 IINA 自己的服务器下载，没有版本承诺，排除 |
| **D. 一次生成、固定、镜像** | **推荐** |

方案 D 的做法：

- 新建 workflow `deps.yml`，只能手动触发。
- macOS job 跑在 **`macos-14`** runner 上：
  1. `brew install mpv ffmpeg`；
  2. 按 §3.3 收集闭包（libmpv、ffmpeg、ffprobe）并改成 `@rpath`；
  3. 生成 `deps.json`（配方、版本、许可证、源码 URL 和 sha256）；
  4. `brew fetch -s` 拿到每个配方的源码，打一个源码包。
- Windows job：把 shinchiro 的 mpv-dev、BtbN 的 ffmpeg 原样转存。
- 产物上传到 `vestrasys/vestra-player-releases` 的一个预发布 release `deps-<日期>`。
- 播放器仓库的 `src/packaging/deps.json` 记录 URL 和 SHA-256。`setup-libmpv` 和打包 job 都只从这里下载并校验。
- 升级就是重跑 `deps.yml`，再提交新的哈希。每次构建用到的字节完全相同，GPL 源码也一起有了。
- 风险：GitHub 的 `macos-14` 镜像以后会下线，大约 2027 年。到时候要么改用 `macos-15`（最低系统变成 15），要么转向方案 B。

## 7. 接入 release.yml

```text
tag v* / dispatch(dry_run)
verify     版本号与标签一致；doc/releases/v<版本>.md 存在（发布说明的来源）；node.json、deps.json 存在
static → windows / macos（现有的 build-test，不用缓存）
package-macos   [macos-15]  下载并校验 node 和 deps → cargo build --release（MPV_LIBRARY_DIR = deps 目录）
                → cargo about → package/macos.mjs（暂存、拼 .app、ad-hoc 签名、check-licenses）
                → 冒烟测试（env -i，PATH 里没有 node，--headless --seconds 6，检查插件全部 ready）
                → hdiutil ULMO → artifact vestra-unsigned-macos-arm64
package-windows [windows-2022]  同样的流程 → iscc → setup.exe、portable.zip → 静默安装、冒烟、卸载
sign-macos      TODO（environment: release）
sign-windows    TODO（environment: release）
source          git archive 打源码包，加上 deps 源码包的链接或副本（§5.4）
publish         needs 上面全部；environment: release；只在 tag 且不是 dry_run 时运行
                → 用 GitHub App 取 token → 在 vestra-player-releases 上 gh release create v<版本> --draft
                  --target main --notes-file doc/releases/v<版本>.md，上传资产和 SHA256SUMS
```

- **跨仓库写入，权限最小化**：推荐 **GitHub App**。
  - App 只安装在 `vestra-player-releases`，权限 `contents: write`；
  - App ID 和私钥只放在 `release` environment 里（只允许 tag 部署，需要人工审批）；
  - job 里用 `actions/create-github-app-token`（固定 SHA）现取一个 1 小时有效、只对这一个仓库有效的 token；
  - 不依赖某个人的账号，也不用管过期。
  - 备选：fine-grained PAT，只授权这一个仓库的 Contents 读写，有效期不超过 1 年，同样放在 `release` environment。更简单，但绑在个人账号上，还要定期续期。
- 现有的 `draft-release` job 用的是 `github.token`，release 建在私有源码仓库里，要改掉。
- 资产命名：

```text
VestraPlayer-<版本>-macos-arm64.dmg
VestraPlayer-<版本>-windows-x64-setup.exe
VestraPlayer-<版本>-windows-x64-portable.zip
VestraPlayer-<版本>-source.tar.gz            （GPL，§5.4）
VestraPlayer-<版本>-third-party-sources.txt  （或者源码包本身）
SHA256SUMS
```

  未签名的草稿 release 在标题里注明「Unsigned test build」。
- **nightly**：继续用 `_package.yml`，产出改成 DMG 和 setup.exe，作为 Actions artifact 保留 7 天，**不往公开仓库发**。命名 `vestra-nightly-<日期>-<短哈希>-<平台>`。macOS 打包 job 按 10 倍计费，只在 main 有新提交时跑，现有逻辑不变。
- `_package.yml` 加一个输入 `kind: release|nightly`，release 时不用缓存。冒烟测试和许可证检查两种情况都跑。

## 8. 以后接签名要预留什么

- **预留的位置**：
  - `sign-macos`：在 ubuntu 上跑 rcodesign（ci-research §10.2）：
    - `rcodesign sign --code-signature-flags runtime`，由内到外，用 §3.4 的 entitlements，并加时间戳；
    - `rcodesign notary-submit --staple`，`.app` 和 `.dmg` 都要；
    - 签名在 Linux 上，所以打包 job 要上传「未封装的 .app」（zip），DMG 在签名以后重新生成。也可以在 Linux 上用 `hdiutil` 的替代工具，不过更稳的是签名后回到 macOS 再生成 DMG，这一点到时候再定。
  - `sign-windows`：Azure Trusted Signing，GitHub OIDC（`id-token: write`）加 `azure/trusted-signing-action`。先签 `vestra-host.exe`、`node.exe`（是否重签官方已经签过的 node.exe 可选）、`libmpv-2.dll`、ff*.exe，再编译安装包，最后签 setup.exe 和卸载程序（Inno 的 `SignTool=`），然后 `signtool verify /pa`。
  - 两个签名 job 只读取打包产物，不执行仓库代码，secret 只放在 `release` environment。
- **用户需要准备**：
  1. Apple Developer Program 会员（99 美元/年，组织账号要 D-U-N-S 编号）；
  2. 一张 Developer ID Application 证书（.p12 和密码）；Developer ID Installer 证书可选，只有出 .pkg 才要；
  3. App Store Connect API Key（Issuer ID、Key ID、.p8），用于公证；
  4. Team ID；
  5. Azure 订阅，开通 Trusted Signing（约 9.99 美元/月），完成身份验证（个人或组织；组织要求有一定年限）；
  6. Azure 上的 Code Signing Account 和 Certificate Profile；
  7. 一个配置了 GitHub OIDC 联合凭据的 App 注册（租户 ID、客户端 ID、订阅 ID）；
  8. 确定 bundle id（§3.2），签名以后就不能再改。

## 9. 实施计划

| 步骤 | 内容 | 负责人 |
| --- | --- | --- |
| 7.3a | 宿主：`platform::install_layout` 与 `runtime_paths.rs`（§2.4），替换 4 处 node、runner、ui、builtins、ffmpeg 的查找；打包运行时设置 `VESTRA_FFMPEG`/`VESTRA_FFPROBE`，去掉 `NODE_OPTIONS`/`NODE_PATH`；Windows：`windows_subsystem`、`hide_console`、`+crt-static`、图标和 VERSIONINFO（build.rs 里加 `embed-resource`，要过 cargo-deny）；日志写到 `~/Library/Logs/Vestra/host.log` 或 `%LOCALAPPDATA%\Vestra\logs`，插件 stderr 也写进去，按大小轮换；mpv 关掉内置 Lua 脚本（如果批准）；`[profile.release] strip`。单元测试：用临时目录造出两种布局，测解析结果和回退顺序 | 维护者 |
| 7.3b | `src/packaging/`：`node.json`、`deps.json`、`third-party.toml`、Info.plist 模板、entitlements、`vestra.iss`、图标源；`src/scripts/package/{common,macos,windows,update-node,check-licenses}.mjs`；`stage-runtime` 补 SDK 的 LICENSE；补 Vestra 的 LICENSE 文件 | 维护者 |
| 7.3c | `deps.yml`，第一次生成并镜像依赖（要先建好发布仓库和 GitHub App） | 维护者写，协调者建仓库和 App、触发运行 |
| 7.3d | `_package.yml`、`release.yml`、`nightly.yml`、`setup-libmpv` 改成从 deps 镜像下载；cargo-about；冒烟测试；更新 `doc/process/ci.md` | 维护者 |
| 7.3e | nes 插件补 jsnes 的许可证 | nes 插件作者（**唯一的插件改动**） |
| 7.3f | SDK 1.2.0：`engines` 和 `@types/node`；播放器升级依赖，`@types/node` 对齐；CI 的 setup-node 对齐所选主版本；文档里的 Node 版本 | 维护者改，协调者打标签 |
| 7.4 | `release.yml` 用 dispatch 做 dry_run，再打一个测试标签，看草稿 release | 协调者 |
| 7.5 | 真机测试（见下） | 协调者 |

**风险**

1. Homebrew bottle 的最低系统版本依赖 `macos-14` runner，有下线时间（§6）。
2. Hardened Runtime 下 LuaJIT 和 V8 的 entitlements 只有等签名时才能真正验证；未签名的测试包测不出来。
3. Windows 上没有真机。`CREATE_NO_WINDOW`、GUI 子系统、日志都只能靠 CI 冒烟测试和 S6 验证。
4. GPL 对应源码的义务（§5.4）。
5. 包体积：DMG 76 MB，Windows 预计 90–120 MB。
6. 内置插件只有在版本号变大时才会在用户机器上升级（`stage-plugins.mjs` 的说明），发版前要检查每个改过的插件有没有升版本。
7. 依赖包里多出来一些用不到的库（x265、svt-av1 这样的编码器，shaderc、vulkan），只是占体积；要裁掉得自己编译。
8. App Translocation：从 DMG 或下载目录直接运行时，整个包会被搬到一个随机的只读路径。我们不往包里写东西，资源也按 exe 的相对位置找，所以不受影响。

**协调者真机测试清单（macOS `.app`，来自 dry_run 或 nightly 的 DMG）**

1. 从 DMG 拖进「应用程序」。从浏览器下载的话，先确认 Gatekeeper 的提示，然后到「隐私与安全性」点「仍要打开」。**Finder 双击启动**之前，先确认 `which node` 找不到，或者临时把 Homebrew 的 node 改名，确保 PATH 里确实没有 node。
2. Dock 图标和菜单栏的名字都是 Vestra Player；「关于」窗口里的版本、build、mpv、ffmpeg、Slint 版本正确。宿主要不要把 Node 版本也加进 `components` 由协调者定，我建议加。
3. 「插件」页里 18 个内置插件全部是已激活。
4. 本地视频：播放、拖动进度、音量、全屏、侧栏和底栏、画中画（macOS 14 以上）。
5. 字幕：外挂 SRT/ASS 和内嵌字幕。内嵌字幕依赖自带的 ffmpeg。中文字幕用 Noto 字体显示。
6. 播放列表的时长（依赖 ffprobe）、截图导出、片段导出（依赖 ffmpeg 和 x264）。
7. NES：加载 ROM，有画面、有声音、键盘能操作。
8. 商城：从官方仓库安装 `zysilm.youtube` 或 `zysilm.bilibili`（托管在官方仓库的第三方插件），签名校验通过，安装后能用；卸载。
9. 工坊：打开就行（它会启动 `process.execPath`，也就是自带的 Node）。
10. 退出再启动：插件状态、侧栏和窗口位置、播放历史都还在。数据在 `~/Library/Application Support/Vestra/{packages,plugin-data,ui-state.json}`，**包内没有任何文件被改动**（`codesign --verify --deep --strict` 仍然通过）。
11. 用 `ps` 看插件进程，确认是 `Vestra Player.app/Contents/MacOS/node`，不是 Homebrew 的 node。
12. 日志文件生成在约定的位置。
13. 把 `.app` 删掉再装一遍，数据还在。

---

## 需要协调者（或用户）决定的事项

1. **Node 主版本**：26 LTS（推荐：和开发环境、CI 一致，支持到 2029-04）还是维持 24（2026-10-20 已经进入维护期，2028-04 结束；选 24 的话 CI 要改成测 24）。
2. **GPL 对应源码**（§5.4，用户决定）：每个 release 附带源码包和第三方源码（推荐），还是走别的路。
3. **bundle id**：`com.vestrasys.player`（要确认域名）还是 `io.github.vestrasys.player`。
4. **macOS 架构和最低版本**：第一版只出 arm64、最低 14.0（推荐）；以后要不要出 Intel x64。
5. **依赖来源**：用「deps.yml 一次生成、镜像到发布仓库、固定哈希」的方案（推荐），macOS 依赖在 `macos-14` 上生成。
6. **ffmpeg/ffprobe 随包**：macOS 带 Homebrew ffmpeg 的 ffmpeg 和 ffprobe（推荐）；Windows 带 BtbN gpl-shared（多约 35 MB），还是第一版不带。
7. **Windows 安装包**：Inno Setup 按用户安装，加便携 zip（推荐）；卸载时默认保留用户数据。
8. **发布仓库的凭据**：GitHub App（推荐）还是 fine-grained PAT；由协调者创建 `vestra-player-releases` 仓库和 App。
9. **mpv 内置 Lua 脚本和 ytdl 关掉**（§3.4）：会改变行为，但能避免 Hardened Runtime 的问题和意外调用 yt-dlp。
10. **图标**：自动合成圆角底板（推荐），还是等设计稿。
11. **发布说明的来源**：`doc/releases/v<版本>.md`（推荐），缺了这个文件 release 就失败。
12. **文档类型和文件关联**：第一版不做（推荐），之后作为宿主功能单独排期。2026-10-25 已做，见 §3.2「文档类型」。
13. **SDK 1.2.0**（`engines`、`@types/node`）：和 S7 一起发。

---

## 10. 宿主实现（7.3a，2026-10-22）

按 §2.4 实现，有两处与上文不同：`InstallLayout` 直接给出各个目录（不再只给 `resources`），ffmpeg/ffprobe 不是 `Option`（决定 5：两个平台都随包）。

### 10.1 安装布局（打包脚本按这张表放文件）

`platform::install_layout(system, exe, is_file)`（`src/host/src/platform/layout.rs`），纯函数，三种布局在任何机器上都有单元测试。

| | macOS `.app` | Windows 安装目录（安装版与便携 zip 相同） | 开发构建 |
| --- | --- | --- | --- |
| 判定 | exe 是 `<X>.app/Contents/MacOS/<exe>`，且 `Contents/Info.plist` 存在 | exe 同目录下有 `resources\runtime\runner.ts` | 以上都不是 |
| UI | `Contents/Resources/ui/`（含 `host.slint`） | `resources\ui\` | `src/host/ui` |
| 运行时 | `Contents/Resources/runtime/runner.ts` | `resources\runtime\runner.ts` | `src/runtime/runner.ts` |
| 内置插件 | `Contents/Resources/plugins/builtin.json` | `resources\plugins\builtin.json` | 无（`VESTRA_BUILTIN_PLUGINS`） |
| Node | `Contents/MacOS/node` | `node\node.exe` | `VESTRA_NODE`，否则 PATH 上的 `node` |
| FFmpeg | `Contents/MacOS/ffmpeg`、`Contents/MacOS/ffprobe` | `ffmpeg\ffmpeg.exe`、`ffmpeg\ffprobe.exe` | `VESTRA_FFMPEG`/`VESTRA_FFPROBE`，否则 PATH |
| 许可证 | `Contents/Resources/licenses/` | `licenses\`（不在 `resources` 下） | 无 |

`runtime_paths.rs` 在第一次使用时解析一次。打包运行时：

- 环境变量仍然优先（调试用）：`VESTRA_NODE`、`VESTRA_BUILTIN_PLUGINS`、`VESTRA_FFMPEG`、`VESTRA_FFPROBE`；空字符串等于没设。
- **不查 PATH 找 Node**。Node、runner、`ui/host.slint`、`plugins/builtin.json` 缺任何一个，启动时直接报错退出（有界面时弹一个错误框，列出缺的文件），不会让 18 个插件逐个失败。
- 插件进程：去掉 `NODE_OPTIONS`、`NODE_PATH`，保留 `NODE_EXTRA_CA_CERTS`；随包的 ffmpeg/ffprobe 存在时设成 `VESTRA_FFMPEG`/`VESTRA_FFPROBE`。不加任何 Node 参数（§2.5）。
- 「关于」里的 Node 版本是 info 插件进程自己的 `process.versions.node`，插件进程由上面的 Node 启动，所以就是随包的那个，不用改。
- 开发构建：宿主自己找 ffmpeg 改成查 PATH（原来写死 Homebrew 的 `ffmpeg-full`；Homebrew 的普通 `ffmpeg` 9.0.2 同样有 x264 和 hqdn3d）。插件进程的环境不变。

### 10.2 mpv（决定 7）

`backend.rs` 在初始化前设置 `osc`、`ytdl`、`load-scripts`、`load-stats-overlay`、`load-console`、`load-auto-profiles`、`load-select`、`load-positioning`、`load-commands`、`load-context-menu` 全部为 `no`；较旧的 libmpv 没有某个选项时跳过（它也就没有那个脚本）。宿主和插件都没有用到这些脚本（没有 `script-binding`、`script-message`、ytdl 相关代码）。测试确认 `script-binding stats/…` 在关掉以后失败（不关时成功）。

### 10.3 Windows

- release 构建是 GUI 子系统（`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`），debug 构建保留控制台。
- 子进程（插件的 Node、宿主自己跑的 ffmpeg/ffprobe）经 `platform::hide_console`：宿主的 stderr 不是控制台时（GUI 子系统、或被重定向）加 `CREATE_NO_WINDOW`。子进程得到一个隐藏的控制台，它再启动的进程（插件跑的 ffmpeg、工坊的 Node）继承这个隐藏控制台，也不会弹窗。SDK 1.2.0 的 `plugin.tools.run` 另外加了 `windowsHide`。
- CRT 静态链接：`src/host/.cargo/config.toml` 对 `cfg(all(target_os = "windows", target_env = "msvc"))` 加 `-C target-feature=+crt-static`，不需要 VC++ 运行库。`libmpv-2.dll`（MinGW）用系统的 UCRT，两边不互相释放内存（rsmpv 用 `mpv_free`），所以没有冲突。环境变量 `RUSTFLAGS` 会覆盖这个设置，CI 和打包都不要设它；`platform/windows.rs` 有测试检查构建确实是静态 CRT。
- exe 图标和 VERSIONINFO 没有做：要先有 `.ico`（由打包脚本从 logo 生成），再在 `build.rs` 里用 `rc.exe` 或 `embed-resource` 嵌入。留给打包那一步。
- `[profile.release] strip = "symbols"`。

### 10.4 日志在哪里

- 宿主和插件的日志都是 stderr（插件进程继承宿主的 stderr）。
- macOS：从 Finder 启动时 stderr 被丢弃。要看日志，在终端里直接运行 `"/Applications/Vestra Player.app/Contents/MacOS/vestra-host"`。
- Windows release：GUI 子系统没有控制台。要看日志，在 cmd 里运行 `vestra-host.exe 2> host.log`（重定向的句柄 GUI 程序同样能用，插件的输出也进这个文件）。debug 构建直接打印在控制台。
- §9 7.3a 提到的日志文件（`~/Library/Logs/Vestra`、`%LOCALAPPDATA%\Vestra\logs`，按大小轮换）**这次没有做**：把宿主自己的 stderr 改写到文件需要 `dup2`/`SetStdHandle`（unsafe），或者把所有 `eprintln!` 换成日志库，改动面大，单独排期。
