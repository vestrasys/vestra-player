# Slint界面接入可行性评估（阶段B）

2026-10-17。API maintainer只读评估；`src`保持阶段A冻结。依据锁定Slint/Slint interpreter 1.16.1的本轮下载官方crate源码，以及B1产品交互/menu-map；以下候选均未实现或通过产品交互验收。

结论：嵌入播放栏与可隐藏侧栏可用标准Slint源码组合实现，不需要终端用户Rust编译器，不必取消sidebar，也不需要另一套布局DSL。未知包首次接入可能整体编译/重建shell；首次准备的明确忙碌反馈、状态恢复与短暂闪动，必须和日常show/hide、菜单驻留、连续调参区分。用户已接受前者的有限代价，不能据此放宽后者连续播放要求。

## 1. 已核到的稳定性边界

- interpreter公开`Compiler::build_from_path/build_from_source`、`ComponentDefinition::create`、root property/callback及global property/callback反射，能在运行时解释Slint，包无需Rust编译。公开global支持别名，适合隔离同名包内桥接。
- 当前`slint::ComponentFactory` re-export实际带`doc(hidden)`和deprecated，原因文字为“Experimental type was made public by mistake”；interpreter的`create_embedded`/`create_with_existing_window`也都是hidden且需`internal` feature。因此不能把工厂类型“能import”说成稳定动态嵌入承诺，也不采用internal接口绕过。
- `ComponentDefinition`内部为`Rc`，编译器import loader也用`Rc`；没有已核到的稳定序列化/跨线程转移已编译组件接口。官方说明编译future通常只有import loader真正异步时才会挂起。改成async/spawn_local不等于CPU编译已离UI线程，不能把非Send对象强行跨线程。
- 公开`Window::show/hide/on_close_requested/size/set_size/position/set_position/set_fullscreen`足以实现非模态独立工具、关闭转隐藏及基本几何恢复。窗口置前/跨窗口系统焦点恢复不由show文档保证，仍须在当前平台验证，不能暗用内部window adapter或第一方unsafe。

上述是官方源码事实；下述组合与状态流程是基于这些接口的设计推论，尚需集中审查后最小实证。

## 2. 推荐的有限嵌入路线

宿主生成普通`.slint` shell，按注册声明import插件包导出的组件，用Slint自己的HorizontalLayout/VerticalLayout/条件可见性布局，仅保留本轮两个挂载位置：底部常驻栏、画布旁可隐藏侧栏。槽位名是通用能力，组件选择由声明/资源句柄决定，不比较插件ID执行业务；冲突返回公开错误。

插件的嵌入组件导出普通Slint组件（非Window），在自己的Slint文件中声明可导出的global用于状态与回调。shell把每个包的组件/global import成宿主生成的唯一别名，并export其global。Rust经公开`set_global_property/set_global_callback`桥接到对应插件实例的TS，插件仍直接编写Slint widgets/layout。这里生成的是Slint import/组件引用和有限布局，没有一套JSON控件树、动态CSS或另一个通用UI解释器。

同一插件的权威业务状态在TS/公开宿主资源，global是其呈现副本，不能拿global作为跨插件共享对象。界面重建后重放状态并重新绑定回调；所有句柄仍受实例generation约束。宿主可以读多个包以组合shell，但插件文件不得相互import；生成的shell不成为插件间业务通信后门。

这个路线的代价是嵌入组件集合变化可能重编整个shell。原生主窗对象可重建，但libmpv、媒体session、插件进程和任务资源不能跟着重建。`create_with_existing_window`不是此路线的稳定依赖；保存几何/可见性/焦点目标后以新shell对象恢复，允许已说明的短闪。

B1最终已选择列表仅作为可隐藏sidebar；无需实现自动转独立窗口。较早转窗设想在原则上应是同一业务状态、两个独立呈现实例，不应承诺原组件的任意reparent；它现已不属于本轮需求。

## 3. 首次准备、日常显示和失败恢复

建议把准备状态和显示状态分开，而非一个`open`承担所有隐式工作：

1. 安装/首次激活未知UI：校验包路径和版本，检查Slint导出/桥接类型，登记候选；当前可用包及shell保持有效，先显示“准备界面”和候选状态，再开始组合编译。不能先隐藏旧shell才发现新包有语法错误。
2. 编译可先在独立辅助进程做诊断以避免无效包影响主窗，但它不能把非Send的已编译对象传回；正式UI实例所需编译成本仍须诚实测量。阶段A没有这个预检事务，需后续实现，不将其当现有能力。
3. 提交前保存窗口几何、栏/侧栏用户偏好、当前选择/滚动、目标控件及插件呈现快照；候选创建、绑定和状态重放成功后再替换旧shell。失败保留/恢复旧可用shell，错误指向候选，旧播放不重载；事件订阅在重建期间不能重复执行业务命令。
4. 形成已准备状态后，日常菜单打开工具、显示/隐藏侧栏、全屏布局变化只操作已编译定义/存活对象或Slint可见性。不要再次解析整个插件包、重跑activate或创建另一播放会话。隐藏工具保留对象和效果；停用才释放其资源。
5. 独立工具可在首次激活准备好定义/实例但先隐藏；首次点击不再编译。若采用惰性准备，首次点击必须先进入明确准备阶段，完成后才属于常规show；不能把每次打开都伪装首次准备。

不能承诺任意体积/复杂度Slint的固定时限。对本轮有限包登记源码/资源量，实测冷准备与缓存后显示分别耗时及最长可见帧间隔；编译失败/候选取消不得覆盖旧版本。真正CPU编译开始后的抢占式取消目前未有稳定接口证据，取消可标“请求取消、丢弃候选”，不能提前显示已停止全部计算。

## 4. 最小尺寸、关闭和焦点

B1的主窗1100×720、最小720×480，侧栏展开门槛960×520、侧栏目标280/最小240、视频区640×360可用Slint布局约束表达。以实际字体、系统缩放和边框后测量；不能只把min-width/min-height写上就宣称每个控件可达。

侧栏展开条件应按实际画布/栏/边距所需空间计算；不足收起保留队列与选择，公开实际可见性及原因，和用户期望显示偏好分别保存。布局重排不应改插件业务状态。底栏跨整个窗口；其两个有限挂载槽不扩成可停靠编辑器。

工具close回调返回保持进程的语义，并hide/发可见性事件；普通hide不会dispose或撤销效果。重开绑定同一个窗口资源。焦点在Slint内可用FocusScope等控件能力；跨系统窗口raise/恢复须补一个当前平台实证，若现有公开接口不满足，再单独评估成熟safe窗口适配，不先承诺所有平台。

## 5. 菜单和原生文件选择的连续性

libmpv工作线程继续运行只证明后端；若Slint Timer/draw因菜单或系统modal loop停住，仍违反产品要求。菜单模型、enabled/checked、快捷键和右键投影必须共用命令事实，但只改变菜单状态不能重编shell。

优先采用Slint已有Menu/MenuItem/ContextMenu能力，菜单不能调用阻塞的业务动作。注意当前winit后端在macOS/Windows会接muda原生菜单，不能把写了Slint Menu误当作一定使用非阻塞Slint popup。1.16.1解释器没有核到公开的`no_native_menu`设置入口；编译器内部配置存在该字段，不等于允许应用依赖它。因此实施前要用当前后端先做菜单驻留最小实证，记录屏幕draw与声音；不能凭API名字宣布不会freeze。

产品不要求macOS原生菜单。若标准菜单路径实测冻结，应评估由Slint公开PopupWindow/标准widgets承载有限菜单，或成熟Slint后端选项；这只是呈现同一命令树，不能趁机自造布局DSL，也不能用internal配置偷过。此处尚无冻结复现，故现在不构成需要用户裁决的路线否定；验证失败且稳定safe替代确实不可行时才提交具体取舍。

文件选择仍用真实系统原生对话框。成熟`rfd::AsyncFileDialog`公开支持macOS/Linux/Windows、Future结果和parent；这是候选而非当前依赖。应保持Slint事件循环，不能在UI线程block_on等待文件选择。异步调用仍不能替代实际打开/停留/取消时的画面与声音观察；平台主线程/parent原生句柄适配需按选定库版本核对，第一方不得加unsafe。

## 6. 集中审查后的最小验证门槛

- 两个测试包分别提供栏与侧栏，用标准import/global别名组合，TS回调可用；独立停用一个只移除自己的UI，另一个及长期媒体session继续。
- 首次安装一个此前未知的有限组件：准备反馈先呈现，记录实际编译/替换/恢复耗时，布局/选择保留；失败候选不破坏旧shell；session/backend/插件实例均不变。此项允许已说明短闪，不虚构任意包时限。
- 已准备工具show/hide与侧栏展开收起重复操作不编译、不重跑activate；主窗最小尺寸和系统缩放下操作可达。
- 真屏幕菜单驻留10秒、连续调参5秒、原生文件选择驻留及取消，分别观察视频像素、音频和session；后台帧计数仅辅助。记录最大画面更新间隔及操作前后状态，以B1承诺判断，而不是只测RPC返回。

本轮无需因此请求用户变更原则。若实现后出现“稳定公开safe Slint无法满足日常菜单/界面连续性”，或只有internal/第一方unsafe才能解决的证据，停止受影响承诺并按工作流提交具体方案与代价；其余设计继续。B1其余B-04..09的来源事务、效果所有权、任务/AV/安装事务仍要在各插件usage集中审查，不包含在本篇UI可行性结论中。

## 官方依据与版本定位

以下源码均指锁定1.16.1；在线latest可能不同，不以新版本文档替代已锁版本：

- [Slint 1.16.1 lib.rs](https://docs.rs/crate/slint/1.16.1/source/lib.rs)：ComponentFactory的hidden/deprecated re-export和global实例边界。
- [Slint interpreter 1.16.1 api.rs](https://docs.rs/crate/slint-interpreter/1.16.1/source/api.rs)：Compiler、Rc定义、hidden embedded构造、公开global反射及global alias测试。
- [Slint core 1.16.1 api.rs](https://docs.rs/crate/i-slint-core/1.16.1/source/api.rs)：公开Window show/hide/close/geometry/fullscreen。
- [Slint winit 1.16.1 muda.rs](https://docs.rs/crate/i-slint-backend-winit/1.16.1/source/muda.rs)：native menu路径；只据此识别风险，不声称已复现冻结。
- [rfd AsyncFileDialog](https://docs.rs/rfd/latest/rfd/struct.AsyncFileDialog.html)：成熟原生异步选择器候选，具体版本待实现时锁定。

## D阶段实际全屏接缝：固定布局反例（待修后原生验证）

V10真实CUA发现进入FS后外区大片白色/左缘残影，Slint reported fullscreen=true但root逻辑几何仍960×600；不能宣布全屏布局通过。`Window::is_fullscreen`锁版实现读WindowItem full_screen属性，set先写属性再推后端，并非OS转场/几何完成fence。公共状态必须区分请求/报告值与实际可用内容几何；Cocoa stylemask也不能作为所有合法borderless fullscreen的唯一判据。

维护者与helper只读锁版1.16.1官方发布源码定位：`i-slint-compiler/layout.rs`239起，width/height binding被标fixed并设min=max；`passes/lower_layout.rs`1645起check_window_layout/adjust_window_layout明确将root.width/height引用移向fixed-width/height常量。原host.slint恰用了`width:960px;height:600px`，这解释OS表面扩大但内容引用仍固定的现象。运行时正常resize应经过winit Resized→公开WindowEvent::Resized更新WindowItem；不能靠手动伪造viewport绕过固定布局。

最小修正候选为根Window使用preferred-width/preferred-height和必要min尺寸，不绑定width/height，不加固定max。普通尺寸请求使用公开safe Window.set_size，再从Slint根几何及关联NSWindow content/frame观测确认；不是改Policy输入冒充resize。此处记录源码依据与候选，尚未修后实际验证。当前GUI冻结时不并发改源码，不把原32单测/Compiler成功等同全屏像素正确。
