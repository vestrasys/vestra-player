# 双独立 runtime provider 控制：有限实证通过

2026-10-18。最终同一明确冻结 binary 的第三 run `e4Qo9X`，host/独立外层消费者均 exit0。provider PID13145/owner1，controller PID13159/owner2。两包不相互 import、不读对方结果、不使用额外消息通道；provider只读自有producer/public source，controller无producer.av能力。harness在宿主结束后才汇总各自JSON，确认owner不同与原requester Book身份。未构建、GUI或改共享源码；host结束已即时交还设备/binary冻结。

## 原始材料与两个消费者错误

- `first-observation-race/`：GfFTgC 完整原执行脚本/插件/stdio/command/results。provider错误地将先source.state、后producer.state当原子快照，旧intent1与新intent2比较失败，自行断开传输；不归因宿主，不标通过。
- `second-retirement-race/`：3aNsmX 完整材料。controller原目标全部通过，但provider先读旧来源、随后producer已因timeout回收，未处理预期unavailable而退出，未执行迟到ACK。
- `passed-run/`：e4Qo9X完整实际执行脚本、两个隔离插件、原始结果/stdio/command、自有replacement.wav、posthoc-identity-checks.json。两次仅修owned消费者跨RPC观察/生命周期错误，宿主/SDK/binary未变；原Book、错ACK、实际5秒timeout、迟到ACK/资源/后续来源目标未降。

第二次窄修仅在第三Notice已故意withheld且producer.state unavailable时重读公共source，必须是带timed-out原因的producer Error，或更高generation的新媒体，否则仍失败；没有泛catch不可用直接通过。

## 实际成功与失败语义

原controller操作101、102均 submit/readback Pending，分别在232/247ms变Completed，terminal ticket保持原requester2/operation对应stamp与cycle；provider1自己的openProvider ticket属于owner1。harness对Notice的producerId/stamp/cycle/paused、ACK完整control、正常terminal ticket、baseSample与当时公开sample position作事后独立exact校验。该补校验只读取已生成JSON，不重跑host。

第一pause Notice epoch2/baseSample2048/pausedtrue，先错baseSample+1 ACK确stale，pending原Notice仍在，再正确ACK；重复正确echo仅原receipt重放。公开pause位置=.042666666666666665秒，180ms后不变。resume Notice同epoch2/base2048/pausedfalse，完成后实际样本位置推进至.091875秒。

第三pause epoch3/baseSample4410/pausedtrue，provider故意不ACK。原op103 Pending真实持续5070ms后Failed，错误为provider frame-bound control timed out；公开source保持producer generation1/intent4/cycle1，phase Error、pausedtrue、position=.091875，之后仍不推进；cap pause/seek/rate/restart均false。host.error producer-failed实收一次。设备health保留ok/错误0/overflow0；这仅本次观察，不取消原tmP43M的活跃Xrun历史。

controller随后打开自有2秒WAV，实际公共source为media generation2/intent5/cycle2、Ready、pausedtrue、position0。provider仅观察该公开Ready后发送未成功ACK过的旧Notice，得到unavailable；随后source stamp/path/phase不变，controller的op103再次查询仍原Failed。自有producer.state unavailable、socket路径ENOENT，binary消费资源不再可用。最终没有依据“进程活着”推控制成功。

## 证据范围

使用真实两个Node插件进程、公共JSONL/二进制PCM与RGBA发包、自有有限数据、真实设备样本消费和超时逻辑；不是callback mock或插件间专用通信。前两ACK刻意200ms有界等待使Pending可观测，只是协议发包边界，不证明NES核心真frameboundary。无物理声学/硬件buffer drain/真实键盘/NES画面/长稳定性或所有故障竞争保证。producer登记撤销及socket删除已证，不由此声称独立waitpid或硬件清空fence。本报告不代表完整M4或产品NES验收通过。
