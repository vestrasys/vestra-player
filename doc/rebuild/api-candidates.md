# Stage D候选共享接缝（仍非共同基线发布）

当前`host.capabilities.apiVersion`仍`vestra.experimental/0.1`；新增设施通过experimentalCapabilities/manifest capabilities发现。所有owner由宿主实例身份注入，不能指定其他插件owner。不识别插件名执行业务。源码/SDK是可运行候选，不表示P1全部通过。

## 来源

`host.source.state()/submit(intent,operationId?)/operation(id)`见`src/sdk/source.ts`。generation标来源替换；intentVersion仲裁手动打开/重播/关闭/控制和列表旧auto-next；cycle区分同源重播及自然结束后seek恢复。实际ready/error来自mpv playlist entry关联；保末帧EOF跃迁按真实当前entry去重。accepted/completed表示命令提交完成，不等于媒体ready。open失败会报告error，不承诺保旧来源。

每owner最多128 pending和512终态操作，正safe integer id递增；相同id相同请求重放，不同请求冲突；淘汰后unknownExpired，绝不盲重试副作用。SDK显式id非法不污染自动计数。自然结束→seek成功进入新cycle Ready，pause不被seek擅改，随后第二EOF可独立报告。独立来源证据和反例保存在`source-evidence/`。

## 处理资源

```ts
await host.processing.replace({
  stage: "video", resourceId: "tone", revision: 1,
  expectedGeneration: state.stamp.generation,
  nodes: [{kind:"eq", brightness:0.1, contrast:1, saturation:1, gamma:1}]
});
```

返回`{stage,revision,generation,status:"applied"}`。revision作用域owner+stage+resourceId，首次token必须递增，现已接有界同参重放/异参conflict（见下）；资源记录在调用owner寿命内持续。真正关联当前load ticket/entry的FileLoaded会重放全部owner最新typed参数，包括runtime修改和同generation restart；loading先置waiting，重放失败置uncertain，不能由旧Ready确认新源。更新必须来源ready，ended暂不支持更新。资源id非空≤128bytes；所有数值有限。完整范围见`effects.rs`与`processing-feasibility.md`，不是任意FFmpeg字符串入口。未知字段拒绝；wire用gainDb/maxGain等camelCase。manifest capability分别`processing.video`、`processing.audio`；state要求`playback.read`。

`state()`另含`source:{generation,phase,load:{entry,generation,cycle}|null,stages:{video|audio:{status:"waiting"|"applied"|"uncertain",error}}}`；load.cycle是实际load周期，不冒充后续seek恢复周期或帧来源fence。`state()`含video/audio的`{owner,revision,resource:{manifestId,resourceId,nodes}}`以及`backendUncertain:{video?:reason,audio?:reason}`。排序由manifestId/resourceId确定。逻辑撤owner与实际清理失败分开，uncertain阶段拒绝更新至重启宿主。错码：invalid、stale、unavailable、disabled、backend、backend-uncertain。applied不是像素visible fence/设备PCM连续性保证；整图重配/失败恢复可能重置DSP历史。裸`playback.set`不提供owner恢复语义，不作为产品工作台实现绕行。

## 命令

`host.commands.register(id, revision, declaration, state)`；`update(id,revision,state)`；`snapshot()`；`execute(id)`；`onExecute(listener)`。注册声明`{title,path,shortcut:null|string}`，路径2..6段，宿主当前配置域顺序文件/播放/视频/音频/字幕/视图/模拟器/插件；每实例id独立但全树path唯一，叶/组前缀冲突拒绝。每owner最多64、全树512。

state完整替换`{enabled,checked:boolean|null,busy,reason:string|null}`，revision递增；禁用/busy不能执行。真实执行直接送该owner的`commands.execute {id,revision}`，不广播，不要求events订阅。公共execute仅执行caller自己注册的id；宿主菜单可解析全树当前key。owner停用自动撤所有command，新实例同id不继承旧路由。Registry错误目前code=command，格式错误invalid。

主菜单/右键同snapshot，当前已接层级MenuView（标准Slint ListView/Button/FocusScope）。每次输入先仅投递带shell epoch的内部事件，下一UI tick才更新层级、关闭/路由，防止Slint回调求值栈内销毁自身。V7同步关闭曾造成真实SIGSEGV，原始报告保留在`composition-evidence/shared-menu-native-crash-v7/`；V8独立真实同路径复验及模型/显隐/键盘/失败保旧范围见[独立记录](composition-evidence/independent-native-v8/review.md)。shortcut当前仍仅声明记录，canvas F11是有限内置键处理，不能称全局快捷键注册已实现。

新窗口态候选将菜单投影改为主窗内inline overlay，`menu-open`唯一控制显隐，card外TouchArea与Esc均deferred Dismiss；换壳清旧popup事实和交互locks。关联主宿主NSWindow失焦也投递同等关闭效果，不以任意工具窗或keywindow猜主窗。该候选仍待本轮真实原生验证，不将V8的Popup证据自动套用。

## 数据与窗口

`ui.open/set`扩展JSON数组→Slint Model、对象→声明匹配的Struct；有限primitive/array/object/null，深度8、总节点4096、单数组1024、字符串/字段名64KiB、累计文本1MiB，非finite/未支持类型/会规范化改名的字段显式错误。必须匹配Slint组件公开属性类型；无图片传输或新布局DSL。模型整体替换，业务stable rowId由普通字段携带，不承诺任意Slint局部滚动/焦点自动保留。回调参数同样有界转换，不静默丢成null。

当前主Host已接动态TS公共路由：`ui.mount({slot:"bottom"|"sidebar",file,component,global,properties?})`、`setMount(property,value)`、`showMount(visible)`、`unmount()`，capability `ui.mount`；每owner一个挂载、每slot一个owner。真实例包在`src/examples/mount-demo/`。普通Slint import组合复用现有主壳、同一视频、菜单与诊断，非另一个实验窗口。首次未知组件prepare在UI线程编译（不把async误称off-thread），有准备反馈、队列最多8且每owner最多一个pending。成功替换保播放后端、来源、插件实例、已确认global/model和窗口几何；任意局部滚动/焦点不保证自动保留。日常set/show没有重新编译。初次准备目前仍有普通RPC10秒timeout/结果未知与查询缺口，尚非安装事务。

每次成功壳有epoch，候选或旧壳回调不路由当前贡献；回调队列64、try_send满明确诊断拒绝，不作为可靠最终滑条提交。回调仅void，参数由有限JSON桥转换。未知组件编译失败保旧壳；停owner立即撤logical贡献、隐藏旧槽、丢回调，再尝试重编剩余贡献；剩余包损坏不能让死owner UI继续存在。编译失败发`host.error ui-owner-cleanup`且宿主stderr持久诊断；V8已实见即时隐藏/旧底栏仍可操作，但当时未独立实收错误。V10补验确认订阅已存在仍未收到，定位重复disable清掉系统id0重编：不能将该次隐藏追认为真的坏源码编译失败。当前已修重复停用幂等，保首次排队清理，等待修后单故障原生复验。

### 窗口态候选（有限原生验证已完成）

`window.state()`/`window.setFullscreen(bool)`需要`ui.window`；set只回accepted(null)。当前state.fullscreen是Slint报告属性，**不是OS几何/转场fence**；V10仍见固定960×600内容与FS白外区失败，现已改根preferred尺寸/min约束，并修新壳初始scale的LogicalSize恢复，V12独立原生复验已确认正确全屏画面及可变几何；仍不能仅用该布尔宣称OS转场/几何fence。实际布局事实以根几何及对应screen内容范围核对。`window.state`事件包括根Slint逻辑几何、实际扣chrome/普通底栏后的canvasAvailable尺寸、fullscreen/menuOpen/dialogOpen，以及slots底栏/侧栏的owner、ordinary preferred、effective、reason、overlay。`ui.mountState()`查询自有槽；`ui.showMount(visible,"presentation")`仅当前FS侧栏临时覆盖，不改普通偏好；`ui.interaction({focused,dragging,popupOpen})`报告自有槽真实交互，不读取宿主内部状态。停owner清locks；换壳清旧交互locks后由新组件重新报告。

侧栏显示须剩余画布至少640×360，侧栏240..280逻辑像素；普通底栏测组件preferred-height，隐藏占0；FS底栏覆盖画布、header/diagnostic/padding收起，默认3秒闲置隐藏，普通preferred=false优先。menu/dialog或底栏focus/drag/popup锁抑制，解除时从新activity计时。偏好不因不足空间/FS/idle改写。native screen关联仅本进程创建窗口strict diff并随壳epoch更新；显式show侧栏按真实chrome+内容区计算resize请求，按关联screen visibleFrame限制，单位校验与关联不可用明确screenLimitUnavailable，不假称已放大；effective仍由后续实测viewport确认。多屏/实际缩放、全局工具键、拖动超过3秒、焦点恢复等未核范围保留，不能仅Policy单测宣布已完成B1。

## 原生文件面板

`host.dialogs.open({kind:"openFile"|"openFiles"|"openDirectory"|"saveFile",title?,fileName?})`，capability`ui.dialog`，单次一个全局面板。结果`{cancelled:boolean,paths:string[]}`；选路径本身不打开媒体、不覆写文件。headless不可用。RFD0.17.2在Slint主线程创建native任务，独立线程只await。官方依据：[RFD macOS async/threading](https://docs.rs/rfd/0.17.2/rfd/#macos-non-windowed-applications-async-and-threading)、[官方示例](https://github.com/PolyMeilex/rfd/blob/master/examples/async.rs)。

仅dialogs.open不使用普通RPC10秒超时，直到用户选择/取消或owner停用；停用统一reject并清pending。owner离开时已显示native面板不保证主动关闭，结果不交给替换实例；面板仍占全局busy至完成。RFD Option不区分系统失败与cancel，本候选不声称该错误分类已经齐备。V4真实选择/取消及驻留超过10秒已确认；面板截图只裁到面板，不能据此证明面板驻留期间主窗口每帧draw。

## 启动本轮原生共享设施样例

先`cd src/host && cargo build --bin vestra-host`，然后仓库根目录：

```sh
VESTRA_DEMO_MEDIA=/absolute/media.mp4 src/host/target/debug/vestra-host --plugin src/examples/host-ui-demo
```

macOS原生证据使用`node src/scripts/stage-native-evidence.mjs /absolute/media.mp4`，仅将当前binary/runtime/SDK/本样例放到新的临时bundle后经LaunchServices启动，避免本轮Documents manifest打开阻塞。开发源仍在src，应用主host.slint内嵌；runner优先bundle Resources，开发checkout仅fallback。没有复制系统字体。该脚本不构建/安装，仍需已装Node及链接的原生库，不能将其当作完整分发安装事务。Commands或视频右键打开同树；Open native file dialog无媒体副作用；Pause/play使用公开source控制；Processing specimen显示两个真实runtime处理slider。样例是基础验收消费者，不是最终产品插件。安装TS+Slint不要求用户Rust编译器；当前分发/原子安装事务仍待后续M4，并不因开发启动命令含cargo而完成该需求。

## 本轮原生修后边界（V4）

操作者root，独立暂存app `local.vestra.rebuild.shared-ui-v4`，本轮实际日志在`menu-evidence/v4-native/`。root通过CUA确认中文领域已绘出，菜单→真实native面板成功，驻留超过10秒后cancel与选择均真实返回；selected路径为`/private/tmp/vestra-bootstrap-long.mp4`，没有打开媒体副作用。面板截图只裁到面板，不能由此证明面板驻留时主窗口每帧draw。先前V1菜单驻留时前后真实截图clock/frame推进是有限独立观察，不合并成最大gap保证。

真实亮度slider后画面变亮；Pause后位置140.31/frame4208，改brightness为-.3后明显变暗、位置同140.31/frame4210，证明此owner API路径暂停像素有效。该次发生在创建audio/norm图之前，**不冒充首轮复杂图反例已完整回归**。随后四次实际audio slider drag值4.41→-4.57→4.93→-3.09，观察同session1最终160.83/frame4825；没有全程最大drawgap/设备声音证明，也不是严格测得的连续5秒latest-only调度验收。工具close/show保留已确认参数，root关闭两窗后inventory/ps确认host与Node退出；LaunchServices的openExit0不是宿主退出码。

字体修正只通过稳定Slint font import使用本机系统Arial Unicode.ttf、指定family Arial Unicode MS；没有复制/分发字体，不能假设别的系统同路径存在。ICU4X复杂脚本segmentation警告仍在，定位Parley0.8的non-complex ctor；不把glyph已绘出等同复杂文字分行全部正确，不启用隐藏接口或修改依赖内部。


## 有界处理操作恢复：实验路由与独立协议验证

P1每个副作用有操作token/未知查询的语义，在处理资源上用实例范围`(stage,resourceId,revision)`等价实现，避免另加无意义operationId。客户端发送前固定token与完整typed配置；同token同参数返回原成功/原失败，异参conflict，不调用后端。`host.processing.operation({stage,resourceId,revision})`仅查询caller本实例：`{status:"pending"|"unknown"|"unknownExpired"}`、`{status:"completed",result:<原applied回执>}`或`{status:"failed",error:{code,message}}`。原回执generation不意味着当前新来源已应用，必须另读state.source确认；旧token重放不会作用新source。

先做typed解析/单资源validate/尺寸及safe integer校验，再规范化payload入本，禁止保512份任意原JSON。尚未admit的pure invalid不消耗token；admit后source stale/unavailable/backend失败都会保存failed并消耗revision，不能改参复用。无历史且大于高水位query unknown；已淘汰或低于高水位未保留则unknownExpired，replace报unknown-expired且绝不重执行。容量满busy为明确拒绝、不消耗token。

每owner128pending、512terminal、64个生命周期resource key高水位（删除资源不清高水位），owner结束drop。内部Ready重放不产生新操作token。experimentalCapabilities增加`processing.operations/0.1`，manifest沿用stage processing.video/audio。实现/SDK已编译，检查证据见[operation-build](processing-evidence/operation-build/README.md)，独立新操作消费者及受控迁移的原处理消费者均已通过，见[独立结果](processing-evidence/operation-independent/review.md)；不据此宣布commands/UI安装等所有副作用未知恢复都已齐。

迁移0.1实验消费者：重复revision不同参数从旧stale改为conflict；一次admitted badgen revision2仍报stale但token已消耗，下一份参数必须revision3。原typed拒绝保资源、不重load、owner隔离/连续30提交要求不变，不以迁移删除原验证。

本轮窗态反例与修正范围：[V10独立结果](composition-evidence/independent-native-v10/review.md)保留固定尺寸FS白区和cleanup未执行/错误未实收；[初始scale复制反例](composition-evidence/startup-scale-copy-failure/README.md)保留V11未交GUI的809高日志与修正说明。V12启动根1100×720、canvas1084×582是作者日志事实，不代替独立FS/resize原生验收。


[V12独立修后结果](composition-evidence/independent-native-v12/review.md)确认真实OS缩窗space/放大选择恢复、全屏画面无白区、栏内真实焦点/menu抑制、FS临时侧栏与普通偏好分离，以及坏剩余源码真实Parse error→宿主host.error→订阅端实收/旧栏仍可暂停。固定root尺寸及重复disable两反例保留。F11在resize后首按未通过；全局快捷键/输入域、工具焦点恢复、连续>3秒物理drag、最大drawgap和设备音频不因此完成。V12初始布局未完成时单次window-policy Invalidviewport原始记录保留；现已按新epoch首个有效布局前延后发布修正，已测epoch后异常仍报错，未用clamp伪造viewport；尚未为这项单独新启GUI。


## 字幕资源公共路径（实验候选）

有界上传、SRT/派生 WebVTT、选择 CAS、原文 revision 条件、owner 清理与有界 operation 查询已接主宿主和 SDK。完整 wire、限制及当前验证见 [subtitle-api-candidate](subtitle-api-candidate.md)。尚无公开样式 RPC、字体 lease 或任务字幕快照，不标完整 SUB/P1 发布。
