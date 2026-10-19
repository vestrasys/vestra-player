# 字幕工作台 0.1.2

本包是可信 TS 进程插件，使用公开 Host API 与普通原生 Slint Window；Rust 宿主负责字幕解析、资源、轨道和渲染。菜单完整路径为 `字幕 → 字幕工作台… / 加载外部 SRT… / 关闭字幕 / 刷新字幕轨`。不占 bottom/sidebar，不提交播放/暂停、seek、重新打开视频或 source 控制。

工作台提供字幕轨列表与切换、关闭、UTF-8 SRT 文件选择/加载、移除本次运行加载的外部字幕，以及 SDK 宣告可样式化轨道的延迟、字体、字号、颜色、描边、底部边距。正延迟使字幕晚出现。字体只来自宿主固定字体清单；不可样式化轨道显示宿主原因。

## 本轮重建来源

0.1.0 的 src 和 dist 被 FileProvider 标记 dataless，未获取可读副本。本轮 2026-10-19 在 `/tmp/vestra-current-products/subtitles/` 从作者当前会话上下文明确重建 **0.1.1 新 revision**；不声称与 0.1.0 同字节。本轮代码根据可读 `/tmp/vestra-current-runtime-cache/sdk/` 核对并类型检查，没有读取旧 repo、Git、旧实现或兄弟业务插件。

窗口采用 preferred-width/height 与 min 尺寸；ui.open callbacks 仅包含 Slint 实际 callback 名称，不含 property。src/plugins/subtitles 为权威提交位置，本地可读候选用于绕过云文件回收。FREEZE.json 记录逐文件非零大小/SHA-256和同步时点；其自身不包含在自身摘要中。

## 构建和检查

```sh
node build.mjs
node --test tests/behavior.test.mjs
```

默认工具链 `/tmp/vestra-rebuild-toolchain-20261018/node_modules`，TS 5.9.3；可设置 `VESTRA_TYPESCRIPT_TOOLCHAIN`。`VESTRA_SDK_DIRECTORY` 默认指向上述本轮 cache SDK。构建把主文件临时复制到本目录 `.typecheck-*`，仅将 type-only SDK 导入改为明确当前 SDK 路径，运行 strict/noEmit 后删除临时目录；没有本包 node_modules 或 typecheck 符号链接。生产 dist 仅包含 manifest.json（strict 0.1.2/main.js）、main.js 和普通 ui/workbench.slint；无 repo SDK/TS 运行时依赖。

本轮真实结果：严格 noEmit typecheck exit 0；生产 build exit 0；6项 node 行为测试 6/6、exit 0。覆盖独立窗口/菜单路径/回调列表、拖动最新值合并、选择器取消、换源选择器隔离、旧列表轨道id隔离、换源和dispose取消待发样式、真实SRT文件字节分块及SHA-256字段、未知commit使用同operationId查询且不discard。Host double 仅检查插件编排；真实文件读取不代表原生字幕解析或媒体验收。

## 并发与取消

上传以 source generation 隔离，轨道/样式写以 selection stamp 检查；Host 认证 owner，插件不伪造 owner。外部字幕 resourceId 和 operationId 独立；资源移除仅作用于本次运行登记的外部资源。样式拖动100ms合并，写操作串行并检查原轨道/selection revision；不调用播放器控制。源 generation/phase 事件触发刷新，已打开工作台另以1.5秒刷新同源轨道变化，不随每个position事件查询。

取消阻止后续步骤，不能撤回已被宿主接受的写操作或替代系统文件选择器取消；工作台明确提示刷新确认。RPC timeout 查询原operationId，不重放写操作。commit已提交后即使结果未知，也不会discard上传，以免破坏仍在执行的提交；最终owner teardown资源回收由Host负责。未知结果显示operationId并提示刷新。插件停用清理timer和监听器，窗口/资源按Host owner teardown关闭回收。

## 尚未验收

作者没有运行Host、Cargo、GUI或真实音频。新dist已交维护者 `/root/rebuild_packages` 执行真实Slint Compiler→activation；本README发布时尚无结果。仍需在维护者独占宿主中打开菜单，检查真实视频的SRT解析/显示、内嵌轨切换、拖动不暂停/重开、换源/取消、卸载回收、字体与时序。界面 applied 按SDK含义显示，不代表可见帧或精确字体身份fence。

设计文档与旧示例Slint此前读取受dataless阻塞，本轮未将其当作已读或验收依据。当前未提供在线字幕搜索/翻译、ASS编辑或任意系统字体选择。

## 0.1.2 原生编译修正

维护者对0.1.1的首次真实Compiler检查失败，记录 `/tmp/vestra-current-candidates-XD1UR6/result.json`：workbench.slint字节offset2038/3587/3761报 `Cannot convert string to int`。三处为ComboBox.selected字符串参数误当索引。本版保留原功能，改为从各ComboBox的 `self.current-index` 传递int给轨道、字体、颜色回调；不修改宿主或公共Compiler。严格类型检查、生产构建及6项编排行为检查再次exit 0。此修正仍需维护者真实Compiler/activation复验，TS与double测试不证明Slint绑定正确。
