# Raw media jobs 独立有限复核

2026-10-17，独立审查者 `/root/rebuild_bootstrap_review`。root 单点集成并明确最终冻结后，独立运行 `node src/tests/media-jobs-review.mjs --run`。实际宿主正常 exit 0 / signal null，原消费者及独立 PNG 分析通过。没有构建、GUI、并发作者实例或把 root smoke 当独立证据；每次结束后均明确交还冻结；主套件完成后仅追加下文另获授权的 same-source 窄补。

原 work：`/var/folders/c9/l6fw8b9n69d_4030n4dcj2nh0000gn/T/vestra-media-jobs-review-ZKvAA3`。

[run/result.json](run/result.json)、[实际命令/退出](run/command.json)、[消费者原始时序](run/consumer-result.json) 已持久保存。run/ 同时包含完整 host stdout/stderr、执行的 consumer/helper 包、完整 PPM/frames.jsonl、实际产物及固定时刻参考 PNG。测试视频未复制，生成方法保留在新脚本；原路径见 command.json。消费者日志经 runner 汇入 host stderr，不另称有未保存的独立 stdio。

## 快照来源与固定时间

在红角标 A Ready 时，以公开 source stamp capture raw snapshot，固定 time=12。用户路径经 rename、copy B 替换及 unlink 旧 A 后，当前播放器另 open 同路径 B。用原 A snapshot 接受两个 job，再 release snapshot handle，两个已接 job 均完成；release 后 snapshot.state 返回 unknown-expired。

两个实际 PNG 均成功完整解码为 1280×720、163,791 bytes，角标 RGB=[254,0,0]，SHA-256 均为 `51d640fbe2e4f1a827f513317c239d8497eb19ce83fc0a0536109b6d5642e394`。当前软件播放帧则实际为蓝 B，公开来源 generation 2。不是以 current generation 给离线红图贴标签。

更进一步，仅用仍保留的原始 `a.mp4` 副本，以独立 FFmpeg 在 12s 与 0s 分别生成参考，未重新打开已替换的 user.mp4 作为参考。两 job 与 12s 参考全 RGB 比较均 MAE=0 / exactDiffChannels=0；与 0s 参考均 MAE=1.409474464699074、391,279 差通道，排除了管道仅产生任意红 A 帧。详见 [fixed-time-analysis.json](run/fixed-time-analysis.json)，实际产物 [A PNG](run/artifacts/a-output.png)、[第二 A PNG](run/artifacts/a-second.png)、[12s 参考](run/reference-a-12.png)、[0s 参考](run/reference-a-0.png)、[当前 B 播放](run/current-b-playback.png)。这仍不是原生 PTS/来源原子 fence。

## 操作、发布与生命周期

- capture/start 的同参数 token 重放与 jobs.operation 原结果、异参 conflict、旧 source stamp 的 admitted stale/query/replay，以及 helper 独立 owner operation 1 为 unknown，均由公开 API 通过。
- 已有目标是本轮 sentinel，原 bytes/hash 保持。碰撞 job 最终 failed/export-failed，明确 File exists；publicationStarted=true 仍锁存，不能当成未尝试发布。[原字节](run/artifacts/existing.png) 的 SHA-256 为 `97f0f1668690f692ebe2eb2e97318f75efd6898ad518e554c4a88b55bf85e569`。
- job 4 取消返回 requested，最终 cancelled / publicationStarted=false / outcome.cancelled=true；没有发布文件，取消目录为空。本轮实证的是赢得发布边界前的接受取消，未实测 tooLate 竞争。
- helper 两个 job 5/6 在停用前均真实 running / publicationStarted=false；能看到宿主子 FFmpeg 96635 处理本轮 owner-cancel 临时输出。停 owner 后观察实际子进程消失、目录无临时文件/最终文件。停 owner 后公共 job 记录被移除，未声称还能查询其公开 terminal；也未用 jobs.list 不含 helper 当成 worker 已退出的唯一证据。

## 播放进展的精确范围

消费者自动选择的观测跨度是 182ms：当前 source B position 0.066667→0.266667，render videoFrames 5→8；起止实际 FFmpeg child 分别 96561→96590，是两个串行旧 A 导出中的不同子进程，不能写成同一个 worker 持续读完整 182ms。

进一步从未修改的原 timeline 找到同 child 96590 在两个采样点持续可见的 46ms 区间：B position 0.266667→0.3，renderFrames 8→11，job 2 running 且尚未 publishing。该衍生分析完整保存 [worker-same-process-analysis.json](run/worker-same-process-analysis.json)。未来脚本筛选已加强为同 PID，语法检查通过但没有为此重跑或改写本轮原始结果。

以上实证是“播放当前 B，同时 worker 导出旧 A”时的有限进展，不是当前播放 A 与 worker 同读 A 的游标隔离证明，也不是长时压力/连续声音测量。同对象读写边界仍需下一独立有限 same-source 场景；首次报告时该补验尚未运行；随后唯一获授权的补验见下文附录。

## 结论与边界

本轮公开 raw snapshot/job 子集的旧对象/固定时刻 PNG、release 后已接 job、生存 owner 操作恢复、无覆盖发布、接受取消、停 owner 实际清理，以及 B 播放与旧 A 导出并行的有限进展，通过独立真实消费者与实际 PNG 分析。没有发现本轮新增阻断项，可继续必要 M3 集成；不表示 processed snapshot/export 或完整 M3 已通过。

主套件本身未测 processed effects/subtitles/font 固化、同对象播放/worker cursor 隔离、全容量/长期压力、任意 worker 崩溃注入、tooLate 发布竞争、物理显示或设备音频。Lease 是 metadata-checked 文件身份，不是抵抗原地改写/恶意 writer 的不可变快照。

PPM 的 frameSourceAssociation 保持 none，公共 observedSource 是观察快照，角标只是本 fixture 的内容身份。实际 FFmpeg 子进程与公共 running 的采样不是内核 read 系统调用完整跟踪；46ms 结果只作有限进展证据。没有把 Node 存活、state 空列表、操作 accepted 或任意红色输出当完整成功。


## 后续唯一 same-source 窄补（已完成）

root 明确授权继续使用原冻结二进制后，独立执行一次 `node src/tests/media-jobs-review.mjs --same-source`，实际宿主 exit 0 / signal null，coverage=`sameSourceProgressObserved`。原主套件没有重跑，没有 build/GUI 或为延长 worker 设置假 sleep。

原 work：`/var/folders/c9/l6fw8b9n69d_4030n4dcj2nh0000gn/T/vestra-media-jobs-same-source-yzlXEi`。完整 [same-source/result.json](same-source/result.json)、[原始 timeline](same-source/consumer-result.json)、[命令/退出](same-source/command.json)、stdio/执行消费者/PPM、实际 PNG 与参考均已持久保存，未覆盖原 run/ 证据。

这次只有 A 正在播放，snapshot 亦捕获同一 A 对象、固定 time=12，接受两个 raw PNG job，期间不换路径/来源、不暂停。真实同一个宿主子 FFmpeg PID 97143 在间隔 106ms 的两个采样点可见（均 `/dev/fd/0`、`-ss 12`，job running/nonpublishing）；当前 A source 保持 Ready、generation=1 / intentVersion=1，position 0.066667→0.166667，renderer frames 2→5。

两个实际 PNG 均完整解码为 1280×720、红角标 [254,0,0]，与本轮原始 A 副本独立生成的 12s 参考全 RGB MAE=0 / exactDiffChannels=0，均 SHA-256 `51d640fbe2e4f1a827f513317c239d8497eb19ce83fc0a0536109b6d5642e394`。[第一 PNG](same-source/artifacts/same-a-0.png)、[第二 PNG](same-source/artifacts/same-a-1.png)、[独立参考](same-source/reference-a-12.png)。作业最终 completed；实际 encoder 子进程消失，产物目录只有预建 sentinel 与两个预期最终 PNG，没有 worker 临时文件。

该补验填补了“当前 A 播放与离线 worker 同对象 A 读取”的短区间实际前进证据，有限通过，没有本轮新增阻断项；结束后立即交还冻结。它不是完整 read 系统调用追踪、长期游标隔离/压力保证或设备连续声音证明，也不扩为 processed/完整 M3 通过。主套件的旧 A/新 B 身份与固定 12vs0 比较、no-clobber/取消/owner 生命周期结论继续由原 run/ 单独支持。
