# 有限包生命周期独立验证

2026-10-18。只读当前新源码后准备独立消费者；未读旧实现、未修改共享源码、未执行 Cargo、GUI 或设备操作。协调者及唯一维护者明确授权冻结后的 headless 两轮。下列结论来自实际首轮运行，不是静态代码推导。

执行 `node doc/rebuild/package-evidence/independent/run.mjs --run`，工具 session **28691**，外层退出 **0**，工作根为 `/var/folders/c9/l6fw8b9n69d_4030n4dcj2nh0000gn/T/vestra-independent-packages-sflrUc`。两次 host 在消费者写出成功结果后由 harness **SIGTERM** 收尾；各自记录为 `code:null, signal:SIGTERM`，不称 host 自然退出 0。随后进程清单未见该独立工作根关联进程。

宿主、SDK、runtime 先复制至自有临时 `frozen/Contents`，两轮共享自有 store，未读取作者实例状态。9 个冻结文件的 SHA256 在运行前后精确一致。宿主 SHA256 为 `918bbc7a4766b29c370aafc8820c5b6bca935efe55a4e73664b4404784c92450`；完整清单见 [frozen-sha256.json](passed-run/frozen-sha256.json)。临时冻结二进制保留在上述工作根，仓库归档 SDK/runtime、消费者、生产 JS、Slint、日志和原始结果。

| 实际路径 | 观察与断言 |
| --- | --- |
| 校验与试载 | 三个独立生产 `.mjs` 包及普通 `counter.slint` 真实通过校验；试载 owner3 从 pending 到 ready，installed revision 仍0；匹配试载未清理时安装返回 busy；stop 后实际 stopped/cleanupConfirmed=true |
| 安装及 CAS | 1.0.0 激活 owner4 后 committed/revision1/current digest匹配；旧 revision 请求报 revisionConflict；已用请求 ID 改参数报 conflict |
| 坏激活恢复 | 1.2.0 真正注册命令、执行公开命令后抛错，owner5 failed；随后旧1.0.0以新 owner6 重激活，operation failed/restored=true，current及revision1保持 |
| 升级与回退 | 1.1.0 committed/revision2/previous1.0.0；显式 rollback committed/revision3/current1.0.0/previous1.1.0 |
| 回执与重放 | 已commit取消返回 tooLate；首次安装精确重放返回原revision1回执，当前revision仍3；第二host可查询已持久rollback回执并精确重放，不增revision |
| 管理者停用 | 独立 observer 以公共 plugins.manage 停用主manager；plugins.list仅剩observer和counter，已安装 owner8 的公共activation仍ready/running=true，installed revision3 |
| 重开实际行为 | 第二host从同store自动启动已commit JS，真实PID26443。计数器通过公共commands.execute两次→真实onExecute累加→自有日志；本轮计数精确 `[1,2]`。消费者只读行为日志和公共接口，不import计数器实现、不增加宿主后门 |

主要原始材料：[第一轮完整结果](passed-run/lifecycle-process.json)、[重开完整结果](passed-run/reopen-process.json)、[实际命令计数](passed-run/commands.jsonl)、[激活进程记录](passed-run/activations.jsonl)、[最终catalog](passed-run/store/catalog.json)、[运行命令与限制](passed-run/command.json)。脚本parse检查均实际退出0；首个真实运行两阶段直接通过，没有改宿主或降低断言重跑。

本结果仅为有限 headless 公共路径：Slint真实编译，但没有打开原生窗口、点击按钮或验证焦点；不能记原生UI通过。没有执行崩溃中断恢复、目录fsync错误注入、旧版恢复失败、提交前取消或512容量耗尽运行；提交前取消及原生协议作者证据由其独立记录，本报告不替代。512条持久请求回执硬上限是本轮明确实验限制，未声称实现淘汰、无限历史或过期请求幂等平台。所有包按可信代码执行，不是沙箱证明。
