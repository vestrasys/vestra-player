# 包管理实验接口（工作基线候选）

2026-10-18。对应当前 `sdk/index.ts` 的 `host.packages`，capability 为 `plugins.manage`。作者首轮headless及原生Slint协议已通过有限流程，独立复验仍进行中；不因本文存在标为全部验收完成。

托管包manifest包含 `id/apiVersion/entry/capabilities/version`；version为无前导零的三个u32数字段，entry为已构建 `.js/.mjs`。安装器不要求终端用户Rust编译器；开发期tsc与插件工坊的TS编译工具不由安装器代替。传统 `plugins.load` 是调试进程加载，不等于安装或升级；其0.1调试manifest继续严格拒绝version字段；托管manifest的version仅由packages.validate及托管加载入口解析，不能将两种加载入口互换。

```ts
const checked = await host.packages.validate(candidateDirectory);
// 校验绑定被捕获的包字节摘要/当前契约，返回token。
// validate检查生产JS语法和真实Slint编译，不等于TS strict检查。
const trial = await host.packages.trial(checked.token);
// 轮询trialState直到ready/失败；ready包含activate及初始UI完成。
await host.packages.stopTrial(trial.trialId);
// 继续查trialState，直到stopped且cleanupConfirmed，才安装同一候选。
const before = await host.packages.installed(checked.manifest.id);
const accepted = await host.packages.install(
  checked.token, before.revision, requestId,
);
// accepted不是完成：用同一operationId查询到明确终态。
const result = await host.packages.operation(accepted.operationId);
// 后续显式回退使用新的requestId及当前installed.revision。
```

上述是调用顺序，不省略实际等待/错误处理来冒充完整应用代码。网络/RPC超时只表示结果未知，用原operationId查询；不得生成新requestId盲目重发。重复requestId且请求相同返回原操作，参数不同conflict。提交前cancel须等cancelled和恢复/清理完成；提交后cancel返回tooLate，应另用rollback，不叫取消成功。

试载是可信代码执行，不是安全沙箱。当前有限试载只允许commands/ui.window/events，不承诺任意媒体副作用可回滚；同manifest id的trial使用独立运行身份，不能替换已安装实例或访问其资源。停止试载等待实际进程回收和backend owner清理确认，窗口和命令由宿主撤销，不依赖dispose。

安装保存当前与上个成功版，持久安装revision与进程owner分离。新激活失败时尝试重新激活旧包，确认旧版ready才报告restored；旧版恢复失败须failedDegraded，不能显示“保旧成功”。重启时未提交journal由宿主恢复，存储层不自行宣称运行时ready。宿主退出/重启恢复和已提交插件独立运行须看本轮证据，不能由文件存在推断。

实验有界限制：候选最多32、trial记录最多16（当前进程内保留停止回执，达到上限需重启宿主）、同时安装操作最多64、托管包最多128、持久操作记录最多512、catalog最多2MiB。容量满在开始新事务前明确拒绝，不静默遗忘旧requestId重执行。回执与安装变更同次发布；catalog原子替换后的目录同步失败可能结果未知，由宿主读回状态收敛。包库仅由单宿主管理；不是跨进程并发安装平台。

安装目录按包摘要持有，不再引用原候选目录；stage和提交前重新核摘要。GC仅清理不在当前版、上一版或pending journal中的自有包目录。用户文件、下载输出不属于包GC。候选token绑定被捕获的私有快照；源目录后续改变不会替换该token内容，重新校验才产生新token。trial命令在第二级菜单使用“试载 owner”命名空间，普通窗口仍显示包声明的标题；试载窗标题标记及可见点击尚待产品接入。持久操作回执最多64KiB，记录满时保留既有查询并拒绝新操作。。
