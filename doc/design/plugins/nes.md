# NES 插件方案 C2

> 2026-10-18 用户修订：本轮 NES 只要求真实核心可运行并输出游戏音画；下文键盘交互、复位及存读档完整产品要求暂缓，不作为共同基线或本轮交付门槛。通用输入接口保留。存读档与模拟器业务完全归插件，公共 API 只提供通用媒体/输入设施。详见[最新执行范围](../../rebuild/execution.md)。

2026-10-17；C2按集中审查R2与共同契约候选P1窄修；仅设计，未实现、未验收。关联全局 `product-interaction.md` / `menu-map.md` B1、阶段 A `vestra.experimental/0.1`。本文件所有 proposed 接口均待 maintainer 集中审查，不是 SDK 声明。

## 1. 职责与有限范围

- 插件负责真实 NES 核心、ROM 校验、游戏推进、按键映射、复位及单份可移植即时存档；公共宿主负责主 Slint 画布、音频输出、来源仲裁、输入焦点与资源回收。
- 使用成熟 [JSNES 官方 Node 集成](https://github.com/bfirsh/jsnes/blob/main/README.md) 的 `NES` 类，由 TS 插件调用；不用 Browser/Web canvas，不自行实现 CPU/PPU/APU，不用示意图冒充游戏。官方提供帧、音频、按键与状态序列化入口，但时序由集成方管理。
- 核心当前 [Apache-2.0 许可证](https://github.com/bfirsh/jsnes/blob/main/LICENSE)；实施时固定依赖精确版本与完整性摘要，随分发保留许可证、版权及适用 NOTICE，修改文件注明。此阶段不安装依赖。
- 合法样本选 [Thwaite 作者仓库](https://github.com/pinobatch/thwaite-nes)：作者说明其支持 NROM、标准手柄，含游戏音乐。按仓库 GPL-3.0 条件保留样本源码、构建说明和许可证；实施阶段从官方源码新构建并登记 revision、ROM SHA-256、构建命令和许可证，不用商业 ROM、旧缓存或旧测试副本。样本作为独立测试材料，不混作插件代码许可。
- 初版仅该 NTSC iNES / mapper 0 场景、单人标准八键、48 kHz 输出；ROM 兼容和存档可恢复性都需真实核对。排除全 ROM/PAL/多手柄/手柄设备/联网/快进/回放/自动电池 SRAM 持久化。即时存档与 SRAM 不混称。
- 目标是当前 macOS 本机约60帧游戏体验；不承诺全平台、低端设备或精确硬件时序。核心与帧转换在插件 worker，UI线程不跑模拟循环；CPU、音频欠载、呈现丢帧都必须可观察，达不到时上报阻碍而非生成假帧补数。
- 独立条件：只有插件和通用测试宿主也能打开ROM、暂停、输入、存读档、关闭；不需要基础播放或任何兄弟插件。普通播放栏存在时只消费同一公共 source 状态。

## 2. 入口与命令

| 功能/action | 唯一主菜单入口 | 必要副入口 / 行为 | 启用条件与权威状态 |
| --- | --- | --- | --- |
| `nes.open` | 模拟器→打开ROM… | 控制窗“打开ROM…”；系统单文件选择 | 始终可用，单个选择/打开在途；事务状态 |
| `nes.controls` | 模拟器→NES控制… | 显示/聚焦唯一工具窗 | 无ROM也可打开；窗口事实 |
| `source.pause` | B1 播放→播放/暂停（若有该贡献者） | 控制窗“暂停/继续”；均调用公共来源控制 | 当前拥有的ready NES且支持pause；source确认paused |
| `nes.reset` | 模拟器→NES控制… 的上下文 | “复位”；立即软复位，不重开ROM | ready且无恢复事务；核心确认 |
| `nes.save` | 同上 | “保存存档…”；系统保存对话框 | ready且无存档事务；任务状态 |
| `nes.loadState` | 同上 | “读取存档…”；系统文件对话框 | 同上 |
| `nes.close` | 同上 | “关闭ROM”；释放来源，回无媒体 | 当前来源为本实例拥有的NES；source generation |
| 八键映射 | 同上查看说明 | 只在主画布输入域执行 | 输入scope有效且来源ready、未暂停 |

ROM操作集中在模拟器领域，不再放文件→打开或通用工具。B1文件→关闭当前来源若存在，也通过公共close关闭同一来源；NES按钮不调用兄弟插件命令。普通来源期间控制窗显示“当前不是NES”，除打开与查看说明外的动作禁用。

## 3. 界面、焦点与生命周期

独立 Slint 非模态窗口，初始440×400、最小360×300逻辑像素；小屏控件区滚动，状态和取消保持可达；激活插件不自动显示。

```text
NES控制                              [×]
ROM：Thwaite.nes     状态：运行/暂停/错误
[打开ROM…]   当前播放将切换至所选ROM
[暂停/继续] [复位] [关闭ROM]
[保存存档…] [读取存档…]     [取消任务]
输入：点击主画布开始；离开画布即释放按键
方向键：十字键   X：A   Z：B
Enter：Start    左/右Shift：Select
存档路径 / 任务阶段 / 局部错误与重试
```

- 游戏只画在主 Slint 画布，contain、保留比例、黑边；不在工具窗复制视频。主画布空闲提示“点击画面以控制NES”，获得焦点后提示“NES输入已捕获；方向键 / X Z / Enter Shift”。
- 主画布使用物理按键code（KeyX/KeyZ，键帽提示本轮固定），防键盘布局差异；Shift两侧合并为Select，仅两侧都松开才释放。按下与释放分别转换为 `buttonDown/Up(1, ...)`；自动重复down忽略，方向相反同时按以核心原行为处理。
- 失焦、切到工具窗/文本框、菜单展开、系统dialog打开、最小化、来源替换、暂停、复位/读档前均先releaseAll。宿主input scope必须发送不可丢的撤销信号，worker按完整pressed-set清零；不能等未来keyup补救。恢复焦点不重放旧down，用户须重新按键。
- NES画布方向键用于游戏，Space不映射；NES域不触发普通±5秒和Space播放快捷键；暂停按钮始终可发现。F11和显式菜单快捷键按B1执行，打开菜单先释放按键。
- 右上×及适用Escape仅隐藏窗口，保存状态/任务/ROM继续；重新显示不重建核心。关窗不暂停、不关闭ROM，也不自动把焦点抢回画布；按B1恢复来源控件。
- “关闭ROM”撤销输入、停止时钟、丢弃待播PCM/视频、解绑producer、释放worker/ROM/存档临时句柄，确认后无媒体；不自动恢复此前文件。插件停用/崩溃由宿主强制撤销所属资源；不依赖dispose成功。

## 4. 状态、事务与失败

状态：`empty → selecting → validating → attaching → running/paused → closing → empty`；存读档为独立单在途任务，状态queued/running/cancelling/completed/failed/cancelled。菜单/按钮共享action，不重复启动；未知进度显示活动指示，不编造百分比。

- 打开前说明当前播放将切换。dialog驻留时现来源继续音画，仅释放NES输入；取消不改变source/session/pause。读取新ROM至候选核心，限制文件4MiB、校验iNES头/长度/mapper和NTSC限定，失败显示路径/原因并保留当前来源。
- 候选核心成功执行首帧并准备首批音频后，再以打开时公共expected generation原子replace；若用户期间已切来源，返回stale并丢弃候选，不覆盖新来源。replace成功才公开新generation与ready；提交后故障显示来源错误，可重试或关闭，不声称仍在播放旧源。
- 取消选择/校验阶段清理候选；attach若结果未知先查询request token，不显示“已取消”。过期帧/ACK按实例generation、source generation、timeline epoch隔离。快速两次打开只允许最新意图提交；迟到结果只清理自己的候选。
- 暂停公共控制在帧边界确认：releaseAll、冻结核心推进、冻结媒体时钟、清空尚未播放PCM，保留最后画面；继续从同一核心状态/时间基准重新预填音频。窗口、菜单和dialog本身不擅自暂停。
- 复位在帧边界调用核心reset，清输入与旧AV、递增timeline epoch，保持用户确认的paused状态；失败保留可恢复快照并报告，不重置普通视频参数或公共音量。
- 保存dialog期间核心继续。确认路径后在帧边界短暂获取一致快照，随即恢复原运行/暂停状态，再异步写文件；来源切换后可完成该不可变快照，任务始终标记原ROM，不触碰新来源。
- 存档包：formatVersion、固定core版本、ROM完整SHA-256、mapper/region、createdAt、payload校验和及 `toJSON()` 状态；含快照时游戏时间，不携带宿主句柄和pressed keys。保存成功仅在原子发布确认后显示。
- 同目录随机临时文件以排他创建写入、flush/fsync后原子无覆盖发布至新路径。本轮不覆盖已有文件，系统保存面板即使确认替换也不改变此规则；已有路径提示换名重试，自动新名也必须排他发布，命名碰撞则另选新名。发布期间目标被其他进程创建同样拒绝覆盖，不能用先检查再普通rename假装无覆盖保证。取消/磁盘满/权限错误清理临时，既有存档SHA-256保持不变；无原子无覆盖发布保障的路径报不支持，不先删旧文件，不引入目标身份条件覆盖事务。
- 读档dialog期间游戏继续；取消不改状态。限制包16MiB，检查结构、checksum、核心版本、ROM SHA-256、mapper/region；错误/错ROM仅局部报错。候选核心先load相同ROM再fromJSON并验证首帧；当前核心不中途污染。
- 读档提交在帧边界，核对expected source generation与ROM identity，释放输入、清旧AV、切候选核心和新epoch；保持提交前pause。失败/取消恢复原核心、原paused和时钟，不把迟到候选装到新ROM。
- 切回普通文件（包括其他公共消费者打开）使旧producer立即失效：宿主先撤销输入/AV消费，再通知worker停止；迟到包拒收，所有共享内存/句柄与队列有上限并可宿主回收。存档快照任务可继续，ROM模拟不能继续占音频设备。

## 5. 公共API usage与缺口

初版已有：capabilities、独立Slint窗口primitive属性/回调、events、普通mpv playback。初版**没有**外部producer、主画布输入、真实PCM输出、sourcecaps/结构化原子replace、异步dialog、取消/查询；不能用已有playback.set假装控制NES。

下例仅表达必要usage，不冻结命名/签名；实现前由maintainer决定二进制IPC和Slint接入路线。

```ts
const p = await proposed.sources.prepareProducer({
  video: { width: 256, height: 240, format: "rgba8", stride: 1024 },
  audio: { format: "f32le", sampleRate: 48000, channels: 2 },
  controls: { pause: true, close: true, seek: false, rate: false, restart: false }
});
p.onControl(async request => worker.controlAtFrameBoundary(request));
const source = await proposed.sources.replace(p, { expectedGeneration, requestToken });
const scope = await proposed.canvas.inputScope(source, { buttons: eightButtons });
scope.onPressedSet(keys => worker.setButtons(keys)); // 包括releaseAll及序列号
await proposed.sources.control(source.generation, { pause: true });
// worker输出二进制帧/PCM到p分配的有界通道；不塞进JSONL
await proposed.sources.close(source.generation);
```

| 请求 | 所有者/资源/输入输出 | 失败、取消、清理与最小观察 |
| --- | --- | --- |
| NES-01 / B-08 producer与来源控制 | 实例generation拥有producer，宿主拥有source generation和音频设备；格式声明、二进制AV、控制request/confirmed ack；来源公开pause/close能力，seek/rate/subtitles/普通调节/采样暂不支持 | prepare失败不切源；替换CAS、request token查询；close/崩溃强制撤销。只有NES插件时仍可按钮暂停真实音画；有普通栏时同状态，无ID特判 |
| NES-02 / B-02 输入域 | 宿主主画布focus scope绑定source；有序pressed-set、down/up、revoke/releaseAll；插件映射八键 | 菜单/dialog/blur先release，scope撤销后拒旧序列；观察持续按Right开菜单后角色停止，音画继续 |
| NES-03 / B-03/04 选择与事务 | 请求方拥有dialog/cancel token；系统parent、过滤、取消独立结果；source replace/close条件关联 | 取消不改源，超时结果可查询；错ROM保旧源、切源后迟到load拒绝；资源不依赖兄弟插件 |
| NES-04 / B-02 窗口/命令事实 | 实例拥有命令/唯一窗口，宿主公布显隐和焦点转移 | 关工具不关ROM，重开恢复；声明缺能力时明确不可用，不注册虚假可操作项 |

传输与时钟拟议约束：

- `onFrame`核心buffer按锁定版本官方格式转换为明确RGBA8，不假定Int32主机字节序；单帧245760字节、约14.75MB/s。JSONL单行1MiB虽可容单帧，也不作为生产路径；只传描述符、epoch和控制ack。
- 宿主提供受所有权约束的共享内存或本机二进制通道；视频最多3个slot、呈现队列最多2帧，消费者慢时丢最旧未呈现帧，只保最新完整帧。buffer完成/归还协议避免读写撕裂；Slint Image转换/上传需单独实测。
- PCM为48kHz双声道交错f32le，有限值[-1,1]，约384KB/s；10ms块、环形缓冲最大100ms，目标40ms。满时worker停止推进等待消费信号，不能无限排队、阻塞UI或任意丢音频；欠载填静音且计数告警。
- 核心worker按音频消费的样本计数为主时钟，NTSC本轮目标60帧/秒；以整数样本累计决定下一frame，视频PTS与同一音频样本轴关联。单调host时钟只作唤醒/诊断；不同时用setInterval与音频时钟各自推进。
- 暂停、复位、读档均带discontinuity epoch、清队列并重置AV映射；暂停期间无旧声拖尾。设备不可用时显式错误/暂停，不悄悄退成无声演示。音量/静音消费公共输出状态；宿主可用成熟safe音频库适配，无需与libmpv共用内部设备对象，但切源必须无旧声、无重叠播放和设备泄漏。
- 核心buffer复用须复制到已获slot，不能保留裸引用；队列容量、drop/underrun、producer/source/epoch在诊断可见。10分钟仅作该机器该ROM的有限稳定观察：预热后资源有界、内存无持续增长趋势，允许库缓存增长，不以字节完全不增长为门槛；记录欠载与呈现数据，不新增跨机器或固定帧率性能承诺。

## 6. 验收场景（均待执行）

| 场景 | 用户实际操作 | 可观察结果 / 证据 | 独立与组合 |
| --- | --- | --- | --- |
| 入口与真ROM | 两个菜单、控制窗打开按钮各执行；选新构建Thwaite | 主Slint画布真实游戏、音箱音乐、构建/许可证/ROM摘要、native录屏+PCM消费记录 | 单插件必测；组合再测 |
| 取消/坏ROM | 驻留dialog10秒、取消；选截断/不支持mapper | 原源音画继续；取消不换session，错误路径/原因可见 | 普通来源与NES均测 |
| 八键与释放 | 逐一down/up、组合键、重复down；按Right开菜单/切窗/开dialog | 八键核心输入记录、游戏动作及释放；无卡键、不触发普通seek | 实际键鼠+core事件辅助 |
| 暂停/继续/复位 | 控制按钮和普通栏各切暂停；暂停下复位 | 真实画面/音频冻结恢复；复位仅游戏、paused保留，无旧PCM拖尾 | 无基础插件也可操作 |
| 存读档 | 移动后保存、再移动后读取；运行与暂停分别做 | 恢复同ROM游戏状态，文件包校验及原子发布证据、真实画面/音频 | 单插件必测 |
| 保存失败 | 取消保存、选择已有路径、换新名重试、磁盘/权限错误、发布时同名碰撞 | 既有文件SHA-256不变、新名成功发布、临时清理；明确失败/取消，无假成功 | 文件产物与任务事件 |
| 坏档/错ROM/取消读档 | 读损坏checksum/不同ROM档；dialog取消 | 原核心/paused继续，局部错误；不污染当前ROM | 原生操作+核心快照证据 |
| 关工具与关闭ROM | ×/Escape、菜单重开；点关闭ROM | 隐藏期间音画继续；关闭ROM后无媒体、AV/input/worker清理 | 按钮菜单分别记录 |
| 切源竞态 | 读档准备中开普通file；快速开两个ROM；停用/模拟崩溃 | 旧epoch拒收、普通file无残音/按键，失效句柄计数归零 | 组合及宿主独立消费者 |
| 能力边界/性能 | NES期间查看seek/速度/字幕；菜单驻留10秒、运行10分钟 | 不支持项禁用解释；native音画持续、帧/欠载/队列数据；预热后资源有界且内存无持续增长趋势，允许缓存 | 普通栏存在/缺席均测 |

不把headless帧计数、按钮回调成功或自造图案当作真实NES/音频证据。八键全路由可以核心输入观测补足样本未使用的游戏键，但游戏移动/动作和真实声音必须原生操作呈现。

## 7. 审查与版本

C1于2026-10-17提交，NES-01至04已发送API maintainer与交互负责人。C2同日回填R2/P1：有限producer、公共source pause/close、输入撤销与epoch方向获准；二进制实现、队列签名及最小实证仍待共同基线。存档已统一新路径排他/原子无覆盖发布，删条件覆盖事务；稳定观察已改预热后有界、无持续增长趋势，允许库缓存。P1允许成熟safe音频适配但要求公共音量/静音与切源无残音/重叠，已同步。以上是设计方向审查，不是SDK发布或验收通过；独立验收反例与必要公共实证仍待执行。任何共同基线变更先更新本方案；只有集中审查和明确实现任务下发后才写产品代码。
