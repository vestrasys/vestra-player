# 六插件探针独立验收记录

日期：2026-10-17。验收者为独立 agent，不负责插件和公共 API 实现。最终结论：**本轮六探针实验独立验收通过**。冻结实现上的完整检查退出码 0：52 项 TS 测试、1 项真实 Rust 后端测试、67 步真实 Slint GUI 全通过。全部必要问题已关闭，未降低原验收要求；这不表示生产版本或最终 API 定稿。下文先给最终证据，再保留首次失败与修复历史。

## 最终运行、复现与代码一致性

首次获取代码先执行 `./src/scripts/setup.sh`、`./src/scripts/make-sample.sh`；后者生成被 gitignore 的 `src/samples/sample.mp4`。需要可用的 Node（本机25.9.0）、Rust 1.92+、含 libass 的 FFmpeg/ffprobe/ffplay，以及可显示窗口的桌面会话。

本次独立执行命令：

```sh
mkdir -p .cache/evidence/acceptance/final-tmp .cache/probe-evidence/acceptance-final-gui
TMPDIR="$PWD/.cache/evidence/acceptance/final-tmp" \
VESTRA_GUI_EVIDENCE="$PWD/.cache/probe-evidence/acceptance-final-gui" \
./src/scripts/check.sh --gui > .cache/evidence/acceptance/final-check.log 2>&1
```

退出码 **0**。TS **52/52**，0 fail、0 skip；真实 Rust MP4 后端 **1/1**；真实 GUI **67/67**。strict、依赖边界、全第一方 formatter、Rust fmt/build/clippy 和空宿主+9插件 Slint 编译全部通过。日志没有清理失败或伪 crash disposal timeout；预期无效用户输入的诊断保留，不冒充错误消失。

最终 GUI 位于 `.cache/probe-evidence/acceptance-final-gui/`，独立复查 `video-adjust`、`audio-workbench`、`subtitle-workbench`、`capture-export`、`plugin-workshop`、`generated-counter`、`nes-pressed` 七张截图。初始空宿主 sourceKind 为 `none`，音频 UI 正确标注 `6kHz`，真实字幕可见，Pi显示6轮及真实TS诊断，NES输入RGB仍为 `[188,25,0] → [60,171,255]`。窗口关闭后专用 TMPDIR 只剩 Node 编译缓存，**0 个字幕缓存/受管资源目录**。

独立保存 `final-summary.json`、`code-fingerprint.json`、`final-check.log` 于 `.cache/evidence/acceptance/`。运行前后95个源码/测试/配置文件及binary SHA256完全一致：

- 验收源码 aggregate：`5952e7da3c9ac1124b30c5f7ecb531e21be94c6e02f57c98ad799c73fa97fbae`。
- binary SHA256：`fcae029117a6d3d8e00aeab01a4e179c3a12e8c8f975bc5eb3f6495805c6e949`。
- 维护者基线74项逐文件哈希也全部匹配；共享文件规范 aggregate：`2d5b9999b64c002dbb85449e4a2de420f82e43c608ab256976e92d2ac7701f29`。排序元数据勘误有记录，源文件未改变。

下列阶段性“待修复/待复验”描述仅记录当时状态；当前结论以最终矩阵和问题表的关闭状态为准。最终可提交证据已归档至 [证据目录](probe-evidence/2026-10-17/)，包括 [检查日志](probe-evidence/2026-10-17/final-check.log)、[结果与指纹](probe-evidence/2026-10-17/final-summary.json) 以及实际窗口截图，不包含下载依赖、生成MP4或商业ROM。

## 阶段执行与反例历史

运行环境：macOS aarch64；真实 Rust 宿主 `.cache/probe-target/debug/vestra-slint-experimental1`、独立 Node worker、FFmpeg-full 9.0.2、JSNES 2.1.0。以下命令在仓库根目录执行，FFmpeg-full 路径需保留（普通 Homebrew FFmpeg 不带 libass）。

```sh
export PATH=/opt/homebrew/opt/ffmpeg-full/bin:$PATH
export VESTRA_HOST="$PWD/.cache/probe-target/debug/vestra-slint-experimental1"
node --test src/tests/probes/video-adjust/*.test.ts src/tests/probes/audio-workbench/*.test.ts src/tests/probes/subtitle-workbench/*.test.ts src/tests/probes/capture-export/*.test.ts src/tests/probes/nes/*.test.ts
node src/tools/check-plugin-boundaries.ts
node --test src/tests/acceptance/*.test.ts
```

首次五探针运行：25 tests，25 pass，0 fail，0 skip，退出码 0。已读测试源码核实真实媒体处理路径：图像独立解码为像素；音频测量 EQ/响度/人声与正负时间偏移；导出经 ffprobe 检查音视频和时长；字幕检查多个实际时间区间；JSNES 实际执行自制 6502 ROM，按键及存档影响后续像素/PCM。静态边界检查通过，9 个插件目录；该检查不证明运行时沙箱。

独立增加生命周期与快照用例后，首次 4 tests 全部失败，退出码 1。它们揭示作者测试没有覆盖的公共缺陷，不能用原 25 项通过宣称目标完成。

继续独立运行 `composition.test.ts`：正序与逆序加载五插件、停用画面后音频继续操作、异常 worker 退出回收 effect/source/job，共 3 tests 全通过，退出码 0。此证据不代表正常退出 disposer 完成（两者退出路径不同），也尚未覆盖第六个工坊。

继续运行 `validation.test.ts`：4 个非法可选字段/对话框子项全失败（连同父项 Node 统计 5 fail），退出码 1。

## 问题记录

| ID | 范围 | 初次证据与影响 | 责任 / 状态 |
|---|---|---|---|
| REV-001 | I2、S2、C2 | `lifecycle.test.ts` 真实加载字幕到 ready，再卸载，等待 2 秒后新建 `vestra-subtitles-*` 目录仍存在。`Plugin::drop` 阻塞主线程等待进程退出，disposer 等待公共 RPC 回复；宿主无法处理，250ms 后杀进程，字幕 `finally rm` 没执行。 | 已关闭；两阶段卸载、GUI/EOF drain、最终专用 TMPDIR 无字幕缓存，正常清理与异常回收分开验证。 |
| REV-002 | C1 | `sources.open` 显式传 `sampleRate:44100.5` 被接受；`as_u64().unwrap_or(44100)` 将非法显式值当缺省。对应独立断言 Missing expected rejection。 | 已关闭；小数显式拒绝，最终统一回归通过。 |
| REV-003 | C1、W3 | 公共文档承诺 `plugins.remove` 只移除管理安装的生成包，但对 `host.load` 的普通字幕插件也成功删除。GUI 与 headless 都缺少安装来源/调用者检查。 | 已关闭；管理来源与 caller owner 校验，普通基础包不可 remove，生成包正常管理。 |
| REV-004 | C1 | 源码检查：GUI `plugins.install` 提前分支直接调用 `begin_install`，跳过 `validate_params`；headless 则有该检查。未知字段校验不同。 | 已关闭；GUI install 与 headless 都调用公共 validate_params；源码复核和真实 GUI 安装通过。 |
| REV-005 | E2、I2 | `snapshot.test.ts`：实际字幕 ORIGINAL → 创建 processingSnapshot → image A → 覆盖同一路径为 REPLACED → 同 snapshot 的 image B。实际 RGB 不同。快照只复制 filter 字符串，未冻结引用资源；连续截图组会随字幕翻译/替换/卸载改变或失效。 | 已关闭；显式 API-007 / probe-0.3 迁移，改写/删除/卸载/release 后实际像素保持，最后 managedAssets 为 0。 |
| REV-006 | C1 | `validation.test.ts`：`effects.set sessionId:42` 静默变 persistent；`media.start videoFilter:42` 静默忽略；`sources.present pcm:42` 静默忽略并提交 RGB；headless `dialogs.openFile {title:42,extensions:[42]}` 返回 null。4 子项均 Missing expected rejection。 | 已关闭；公共类型校验，五个相关可选字段/正常结果子项最终通过。 |
| REV-007 | C1 | 两个不同 id 的有效插件复用同一 UI component/global，headless `host.load` 都成功，违背文档唯一性及 GUI 的拒绝行为。`validation.test.ts` 对应断言实失败。 | 已关闭；load/install 共享唯一性检查，独立冲突用例与替换回归通过。 |
| REV-008 | G1 | 原生音频界面标注高频 10kHz，但 `filter.ts` 与实际 PCM 测试均为 6kHz。 | 已关闭；最终原生截图明确显示 6kHz，算法未变。 |
| REV-009 | C1 | 独立 GUI report 第一条 playback 是 `path:null,duration:0,sourceKind:'file'`，headless 则为 none。另 `media.status` 健康任务返回 `error:null`，不符合 SDK `error?:string`；独立子项实失败。 | 已关闭；最终 GUI 第一条 sourceKind 为 none，健康任务 error 字段符合可选 string。 |
| REV-010 | C1、I2、C3 | 首次统一 `check.sh --gui` 的旧插件回归有 4 项 stop 超时，exit 为 null；runtime 即时 shutdown 后，正在途 action 的 catch 再发送 ui.patch，新 pending timer 使进程滞留。另字幕测试的 cwd-relative `src/plugins` 在脚本进入 src 后失效。 | 已关闭；关闭后拒绝新 RPC，旧消费者正常退出；路径无 cwd 依赖，完整统一检查通过。 |

独立回归文件归验收者维护，不能由实现者降低预期以消除失败。REV-004 是额外源码差异，不混入四项已执行失败计数。

REV-005 已触发显式候选 `probe-0.3`：受管 `files` 与 filter 文件占位符；root 批准按社区流程迁移唯一受影响的字幕消费者。`probe-0.2` 失败记录保留；新 usage 仍须证明改写/删除原字幕、卸载来源 owner 后，既有快照像素保持，不能通过删掉原先不变性断言验收。

### 第一批修复的独立复跑

维护者重建后，独立执行：

```sh
node --test src/tests/contracts.test.ts src/tests/acceptance/lifecycle.test.ts src/tests/acceptance/composition.test.ts
```

9 tests 全通过，退出码 0。REV-001/002/003 在此代码状态下已修复并独立复验；五探针双顺序和异常退出仍通过，生成包真实校验/执行/失败替换保留旧版也通过。源码新增 `retirement.rs`，正常卸载保留 retiring worker 通信，处理受限的 cleanup RPC，收到 stopped 后回收；这消除了原先同步 Drop 阻塞清理的根因。该阶段不证明 REV-004/005/006 或工坊已验收。

### probe-0.3 资源与输入复验

独立运行 `snapshot.test.ts` 和 `validation.test.ts` 的最初四个字段用例，6 tests 全通过。新的受管资源公开用法证明：裸字幕文件依赖被明确拒绝；改写源文件、删除源文件、卸载资源 provider、在 job 启动后释放 snapshot，都不改变既有快照的实际 RGB。随后补充最后引用释放后 `managedAssets:0` 的断言，等待最终统一复跑。REV-006 的四个非法字段类型已修复并独立复验。`repro-snapshot-probe02.ts` 完整保留首次旧契约反例，仅为历史手动复现，不进入默认测试；旧破坏性用法在新契约下应被拒绝，不伪装成兼容保留。

### Pi 工坊与真实 GUI

独立运行工坊测试两次，均 3/3 通过。已读实际 Pi `Agent` 调用、模型边界的本地 `StreamFn`、工具执行与完整 transcript：六次模型轮次依次写错误代码、真实校验、收到 TS2322 后重写、真实校验、试载安装、结束；生成计数器真实执行。覆盖真实 Slint 错误、类型错误、activate 错误保留原 route、取消、替换、显式回退和工坊卸载后生成包独立运行。没有网络/付费模型调用。

独立执行真实窗口场景：

```sh
VESTRA_GUI_EVIDENCE="$PWD/.cache/probe-evidence/acceptance-gui" node src/tools/probe-gui.ts
```

退出码 0，报告 67 步全部通过。验收者已视觉检查六探针与生成计数器的 7 张关键原生窗口截图，字幕实际可见，面板可读无叠加/截断；处理后截图确实保存为 PNG；Pi 窗口显示真实诊断与 6 模型轮次；生成计数器 count 为 1。NES 按键前后画面取样 RGB `[188,25,0] → [60,171,255]`。原有面板切换不增加 windowGeneration/compositionCount；安装未知 UI 发生重建时视频时间持续向前。输入驱动真实 Slint 回调，未冒称物理键盘/鼠标自动化或操作系统文件对话框人工验收。

随后在完整报告读取中发现 REV-009；GUI 脚本新增初始 none 断言，须在修复后再次运行。音频截图的 10kHz 文字也在 root 修正为 6kHz 后等待新截图。本次 67 步通过只证明当时已覆盖的行为，不覆盖后来补充的断言。

首次维护者统一检查的失败输出已独立读取并保存在 `.cache/evidence/acceptance/first-full-failure.log`。尽管新增探针检查多数通过，legacy 消费者的关闭回归依然阻止发布；按社区流程归为实现回归，不能通过要求旧消费者更换 API 或延长测试等待来掩盖。

## 最终逐项验收矩阵

以下结果全部对应最终冻结实现和本次独立完整运行，不以历史作者报告替代。

| ID | 结论 | 最终独立证据 |
|---|---|---|
| V1 | 通过 | 四画面参数、跨菜单命令及真实 Slint 面板状态。 |
| V2 | 通过 | 预设/复位/错误输入；独立 PNG RGB 变化测量。 |
| V3 | 通过 | 独立 owner 同链、原位顺序与卸载隔离。 |
| A1 | 通过 | 三段 EQ、响度、人声、正负延迟真实 PCM/时间测量；最终 UI 6kHz 与算法一致。 |
| A2 | 通过 | worker 参数实时更新/跨媒体保留；停用只撤销自己，暂停恢复使用公共播放状态。 |
| S1 | 通过 | 真实多 cue SRT、显示隐藏/原译文、样式/同步；原生窗口字幕及实际图像时区间证据。 |
| S2 | 通过 | 可替换 mock provider、失败重试/取消、媒体切换旧结果隔离，卸载缓存清理。 |
| E1 | 通过 | 原始/处理后/含与不含字幕实际 PNG；GUI 保存有效 PNG。 |
| E2 | 通过 | 实际 MP4 视频/音轨/时长，进度取消；换源/改效果后保留快照，受管文件生命周期补充回归。 |
| N1 | 通过 | JSNES 执行 CC0 6502 ROM，非零 PCM、按键改像素；原生窗口 RGB 也变化。 |
| N2 | 通过 | ROM 对话框过滤/取消、独立 null duration 来源、暂停恢复/存档、停用关闭。 |
| W1 | 通过 | 普通独立工坊 worker，真实 Pi Agent + 本地 mock 模型接缝，无付费请求。 |
| W2 | 通过 | 实际 TS2322 和 Slint 诊断、Pi 修复工具循环、真实安装、生成计数器执行。 |
| W3 | 通过 | 取消、校验失败、activate 失败保留旧 route/状态、显式回退、卸载工坊后生成包仍独立执行。 |
| I1 | 通过 | 六插件各自在空宿主独立运行；静态边界及源码未发现跨插件依赖/私有访问/直接通信。 |
| I2 | 通过 | 六插件正反顺序组合、真实生成包加入；撤销画面音频仍可操作，工坊卸载后生成包可用；正常/异常退出及资源清理。 |
| C1 | 通过 | strict TS、全第一方格式、Rust fmt/build/clippy、空宿主及9插件 Slint 编译、manifest/RPC/namespace/值域与真实消费者回归。 |
| C2 | 通过 | 已读宿主/runtime/SDK/六插件；无六探针名业务分支、Pi 不进入 core、第一方 forbid unsafe、owner 明确。 |
| C3 | 通过 | 0.2 失败候选不追认发布；API-001–007 分类决策、0.3 显式迁移、消费者目标/兼容退出条件和最终代码指纹可追踪，无未解决必要迁移或待用户裁决事项。 |
| G1 | 通过 | 最终真实 Slint 67 步及7张关键截图复核；字幕/工坊/生成计数器/NES可见，现有面板切换不重编译，重建保留业务状态。 |
| R1 | 通过 | 验收者亲自执行全套命令、检查真实产物与源码；最终95文件和binary运行前后无变化，维护者基线74文件哈希匹配，已披露未覆盖边界。 |

不将离线 PCM 测量冒充声音实听，不将 Slint 编译或 headless 成功冒充真实窗口行为，不将 mock 模型场景冒充 BYOK/真实供应商互通。当前平台之外的表现未验收。

源码确认的实验边界：FFmpeg effects 链用于文件后端；外部 NES RGB/PCM 当前绕过该处理链。矩阵 N1/N2 未要求所有来源统一处理，因此不扩张本轮标准，但不能宣称已验证全来源滤镜/音效一致性或生产完备。
