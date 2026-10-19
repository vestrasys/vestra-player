# 插件工坊（本轮独立实现）

普通插件，唯一入口为「插件 → 插件工坊…」。不修改Rust、不依赖兄弟插件。

选择专用工作目录，输入/选择JSON需求，点击生成。仅支持 `{"kind":"native-counter","id":"community.counter","version":"1.0.0","step":1}` 和版本1.1.0/step3。Pi官方Agent 1.3.0真实执行工具；只有模型streamFn为本地确定性mock，无凭据读取、无付费模型调用。第一次故意类型错误，只有真实TS2322 file/line/column诊断后才修复。每次生成独立随机子目录；候选含独立main.js/Slint/manifest及自身Host类型，生产代码不引用工坊或SDK运行实现。

固定TypeScript 5.9.3及许可证已随vendor分发，不在运行时安装npm或调用Rust。main.js含Pi及实际用到的依赖bundle，许可证在licenses，registry锁版在package-lock.json；vendor/node类型版本25.9.9。manifest production JS入口。

已验：当前revision4本地strictTS通过；普通Host控制状态回归5项通过（installed异常/timeout释放busy、install/rollback丢答复沿同requestId查询且只提交1次、trial双击只提交1次且unknown不重发）；这些协议stub不是managed验收。真实Pi独立证据：真实Pi 3轮/8工具动作；两版本各经历真实TS2322→clean；独立生成JS通过普通Host callback stub获得1,2,0与3,6,0。证据位于本地 `/tmp/vestra-current-products/workshop/evidence/`（不打包临时证据/工程/node_modules）。这些不是宿主管理或GUI验收。

首真实Compiler证据Wb0Qce拒绝width/min-width与height/min-height冲突；revision3改preferred尺寸并保min，原失败保留。待复验：宿主Slint编译、普通独立Window所有按钮/本地菜单、原生文件选择、宿主真实validate/trial/install/rollback/operation/cancel、试载清理、停用工坊后独立插件仍运行、真实鼠标点击和组合媒体连续性。root/maintainer独占Cargo、GUI及音频；本作者未执行这些。

包生命周期只用当前Host.packages API。安装与回退始终用户点击；事务按operationId轮询至terminal；timeout按维护者承诺operationId=requestId恢复查询，不发新安装。试载使用稳定requestId；首答复timeout直接trialStateByRequest查询原试载，查询再timeout时保留unknown并提供「查询试载」，不会再次申请。两项新增普通Host协议回归证明只申请1次、找回原handle并停止；真实托管试载恢复仍待本产品验收。详见API_REQUESTS.md。

关闭窗口由宿主普通Window生命周期处理；再次菜单打开使用show，不重建任务。当前未证明系统关闭后hide/show焦点语义。日志最多16KB显示，生成事件与诊断保留在候选下；生成取消abort Pi并kill编译子进程，waitForIdle后返回。

开发：`npm ci --ignore-scripts`后`node build.mjs`，不要求终端用户执行。源码main.ts仅type import ../../sdk/index.ts，运行时不引用仓库SDK。`node test-engine.mjs`仅开发验真实Pi和TS，生产不自动运行测试。
