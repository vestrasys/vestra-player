# Processing operation 独立有限复核

2026-10-17，独立审查者 `/root/rebuild_bootstrap_review`。使用 maintainer 明确冻结的真实 `src/host/target/debug/vestra-host`，仅 headless；本轮未构建、未操作 GUI、未使用 mock mpv。两次消费者及宿主均正常结束，exit 0 / signal null。

| 消费者命令 | 原始 work 后缀 | 持久证据 | 结果 |
| --- | --- | --- | --- |
| `node src/tests/processing-operation-review.mjs` | `vestra-processing-operation-review-mMpQQQ` | [operations/result.json](operations/result.json)、[外部核验](operations/external-review.json)、[命令/退出](operations/command.json) | pass |
| `node src/tests/processing-review.mjs` | `vestra-processing-review-IEvZYw` | [retained-processing/result.json](retained-processing/result.json)、[命令/退出](retained-processing/command.json) | pass |

两个目录均保存本轮生成的 consumer/helper 包与 `host.stdout.log`、`host.stderr.log`；消费者的 runner 日志汇入宿主 stderr，结构化断言写入 result，而非另有未保存的独立消费者 stdio。operations 另保存 `helper-results.jsonl`。媒体未复制，生成方法保留在各自测试脚本；原 work 完整路径见 command.json。

新 SDK 消费者实证了：无 Ready 来源的 admitted failure 可查询/重放；成功 token 可查询并按规范化 typed 参数重放，异参 conflict；stale generation 的 failed token 可查询/重放，纯 typed invalid 不占用 token；切到新 generation 后重放返回旧收据而公共资源保持最新 revision，观察窗没有新增 StartFile/FileLoaded；514 个真实 stale terminal 请求触发淘汰，旧 query 为 unknownExpired、旧 replace 为 unknown-expired，最新失败仍可查；64 个 resource key 上限后 busy 不 admission，query 仍 unknown；停用/重载 helper 的 owner 2→3，同本地 token 在新 owner 下为 unknown 并可独立成功。helper 外部结果与宿主观察一致。

原消费者只作已声明的协议迁移：同 revision 异参由 stale 改为 conflict；admitted bad-generation revision 2 留存失败并消耗该 token，之后 30 次连续提交移为 revision 3…32。原 typed serde、双 owner 同本地 ID、撤一留一、拒绝请求保持公共资源、无 source reload 的预期均保留；新增 source 元数据按 Ready/load/stages 语义核验。30 次提交用时 915 ms，source generation/intent/cycle 均为 1，position 0.3→1.233333，render 诊断帧数 10→38，最终 revision 32。

结论：本轮候选的公共 operation/query/retention/owner 协议及原公共 processing 消费路径有限通过，可继续后续集成。没有发现需要继续冻结宿主的本轮阻断项；已向 maintainer 交还冻结。

边界：未制造 pending 队列饱和；busy 此次只覆盖 resource-key 上限。淘汰用 generation-stale 请求，未注入真实后端半失败、验证旧实际效果回滚或 DSP 历史保留。相同收据与公共资源不变不是底层 filter 调用次数的直接测量。帧计数前进不证明真实像素、PCM、设备连续性或来源帧 fence；本轮不把这些列为通过，也未重跑此前独立的五窗口像素证据或原生窗口验收。
