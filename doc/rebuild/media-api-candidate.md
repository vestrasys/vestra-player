# 公共图片任务候选

当前是阶段 D 的 `media.raw-jobs/0.1` 实验候选，不是共同基线或截图产品。SDK 为 `host.snapshot` / `host.jobs`，能力为 `media.read` / `media.control`。raw PNG 已有独立验收；processed PNG 子集已有作者与独立真实输出验证。plain原生字幕、单快照多时刻采样及时间域修正已通过独立有限消费者，正式材料正在归档；ASS/bitmap等原生格式仍未完成；owned 音频偏移/明确lavf音轨/MP4 source0预热已有独立有限验证，见[MP4候选](media-mp4-candidate.md)，不能发布完整 M3。

## 使用

```typescript
const source = await host.source.state();
const snapshot = await host.snapshot.capture({
  operationId: 1,
  expected: source.stamp, // generation + intentVersion，同仲裁点检查
  time: 12,              // 省略时使用接受时的实际媒体位置
});
const job = await host.jobs.start({
  operationId: 2,
  snapshotId: snapshot.snapshotId,
  kind: "rawPng",
  destination: "/absolute/path/frame.png",
});
await host.snapshot.release(snapshot.snapshotId, 3);
// release 撤句柄；已经接纳的 job 自己持有来源租约。
const state = await host.jobs.state(job.jobId);
// 继续查询到 completed / failed / cancelled，queued 不是完成。
```

本例是公共 SDK usage，不是产品插件实现。调用者实例内固定正 safe integer、单调递增的 operationId；capture/release/start/cancel 共用这一本有界记录。相同 typed 参数重放既有 receipt，异参 conflict；`jobs.operation(id)` 可查询未知结果，淘汰为 unknownExpired，不盲目重新执行。返回旧 receipt 不表示旧任务仍在保留范围内。

`snapshot.state(id)` 查询固定来源 stamp、原用户路径、实际容器、文件大小与媒体时刻；不把内部 fd URL 暴露为持久来源路径。来源需真实关联的 Ready/Ended 普通可定位容器与持有对象；仅扩展名不构成能力确认。接受时核完整来源 stamp，防同 generation 的迟到 seek/restart 请求。

## 生命周期与发布

任务为 queued/running/cancelling/completed/failed/cancelled；内部 publishing 投影为 running，并以 `publicationStarted` 表示已越过取消界线。这个标志在发布失败后也保留，状态/结果成对读取。

`jobs.cancel(jobId,operationId)` 返回 requested / tooLate / terminal。requested 与发布 CAS 争同一个边界；请求成功后禁止发布，但必须等待实际终态及清理，不能把回包当已取消。开始发布后返回 tooLate；完成或失败后为 terminal。owner 停用撤自身句柄/公开记录并请求取消未终态工作，不能据记录消失断言子进程已回收。隐藏窗口不调用停用。

worker 在后台以可取消方式串行获取同来源 Reader，使用与实时播放器独立打开的文件对象；仲裁/UI 不等此锁。编码前后检查持有对象元数据，失败丢弃临时输出；rename/unlink 后仍读旧对象，原地写入检测不是对恶意同长同时间覆写的强不可变保证。

FFmpeg 使用固定参数、媒体时刻、原始呈现方向及 SAR 归一生成 PNG，不读取 Slint 缩略图，不烘焙效果/字幕。输出同目录临时文件，完整解码验证后经 `persist_noclobber` 发布；已有用户文件不覆盖。该设施不承诺各平台崩溃恢复或持久化原子事务。

当前限额：全局 16 / 每 owner 4 个快照句柄，8 个 worker、每 owner 2 个未终态任务，128 条 job 记录；终态可被淘汰。PNG 解码预算 128 MiB，stderr 只保留 64 KiB 尾部。快照释放和 owner 清理不销毁其他 owner 的资源。普通编码错误或可展开的 Rust panic 会收敛 failed；进程级崩溃未验证。

## 当前证据

root 接手中断后的集成，all-targets 测试 host 70 / composition-probe 57 通过，Clippy -D warnings / build 通过。仓库内 TypeScript 进程实测阻塞读取依赖声明，终止结果不算通过；同 lockfile 在临时目录安装完整工具链，以原 tsconfig 和 fresh typeRoots 检查实际退出0。

[作者原始证据](snapshot-evidence/raw-author/checks.json)与 `run/` 保存首次公共 PNG smoke：真实 320×180 红图可解码，已接 job 在 snapshot release 后完成；同目标第二次发布失败、已有字节与摘要不变、临时输出回收。不是独立验收。

[独立 raw 报告](snapshot-evidence/independent-jobs/review.md)已验证换源后旧 A 两张 PNG 精确匹配独立 12s 参考、release 后 job、无覆盖、接受取消及 owner 实际子进程/文件清理。同对象播放/worker窄补也通过：同一实际FFmpeg子进程存在的106ms内，A位置前进、渲染帧2→5，两PNG精确匹配12s参考；不扩大为长时压力或声音连续。

作者另以320×180/SAR2源得到640×180 PNG，[SAR结果](snapshot-evidence/raw-author/sar/result.json)保留。旋转测试初次素材未实际写入旋转矩阵，已归档为无效fixture；改用官方 `-display_rotation` 并先验证90°矩阵后，宿主实际触发 libmpv 软件renderer `mp_image_crop` 断言 SIGABRT，[失败现场](snapshot-evidence/raw-author/rotation-host-crash/command.json)保留。当时未运行到PNG导出，该失败属于实时软件renderer。

后续以成熟物理旋转/已消费metadata清理修复，并经[独立四角复验](snapshot-evidence/independent-rotation/review.md)通过0/±90/180、SAR2、重播/切源/效果共存。六个rawPNG实际全RGB匹配各自独立FFmpeg autorotate/SAR参考，原失败没有覆盖。软件画面以方向/比例/黑边判断，不要求不同renderer逐像素相同；不推广任意角度、首帧fence或完整M3。

## processed PNG 接入候选

```typescript
const processing = await host.processing.state();
const subtitles = await host.subtitles.state();
const snapshot = await host.snapshot.capture({
  operationId: 10, mode: "processed", expected: source.stamp, time: 1,
  expectedProcessingRevision: processing.configurationRevision,
  expectedSubtitle: {
    selection: subtitles.selection.stamp,
    presentationRevision: subtitles.presentation.revision,
  },
});
// snapshot.kind === "processed"，SDK 判别联合类型包含冻结摘要。
await host.jobs.start({operationId: 11, snapshotId: snapshot.snapshotId,
  kind: "processedPng", destination: "/absolute/path/processed.png"});
```

同一控制仲裁点核完整 source stamp、处理配置 revision、字幕 selection stamp 和 presentation revision，并要求当前实际 FileLoaded 绑定的 geometry 与处理/字幕应用确认；准备中明确 unavailable，不借旧 video-out-params 冻结新源画布。接受后持有来源、typed 图、外部字幕派生载体和固定字体租约。后续改参/换源/撤除贡献 owner 不改已接受的快照；任务 owner 停用仍取消自己的未终态 job。

当前视频离线顺序是 FFmpeg 成熟 autorotate → typed 视频图（storage 像素）→ SAR 归一 → 字幕 → 按原始时间线 trim。这样 gblur 与实时「storage 模糊后 SAR 呈现」顺序一致；本轮独立 SAR2 对照正确顺序全 RGB exact0，错误先scale顺序 MAE6.881；不以纯 mapper 测试代替。processed 从源0解码，不先 input seek 破坏字幕时间线。

字幕 off 固定为空；选中外部资源固定 canonical SRT/derived WebVTT 各自摘要、内容 revision、样式和字体。内嵌plain subrip/srt已接worker受限提取，待本轮独立验证；ASS/bitmap等明确 unsupported，不静默省略。字体改为随应用的 OFL Noto Sans CJK SC Regular 2.004、单 face0、固定 SHA256；宿主私有目录由同一租约保持。所有 cue 先核 cmap，缺字明确 unsupported，不能把系统 fallback 当租约成功。实时 provider=none、embeddedfonts=false；离线 FFmpeg 字体目录不是排他 provider，因此不承诺任意 glyph/任意字体的绝对选择隔离或逐像素 parity。

样式以实际正方像素画幅设置 PlayRes、1080参考字号/描边、边距与字面 WebVTT，由成熟 libass 绘制；force_style ScaleX/Y=1。PNG不含音频。当前已接owned偏移并在capture核实际backend值与typed总贡献一致，lavf流选择和MP4 source0 preroll的有限路径见MP4候选；MP4独立有限验收已交还，见MP4候选。

迁移：省略 mode 仍 raw，RawCapture/ProcessedCapture overload 返回相应 Snapshot；state 返回判别联合。新增 processing.configurationRevision 与 subtitle.presentation.revision 为聚合配置条件。旧本机 local-arial-unicode fontId 不再列入当前字体目录，应从公开 fonts 获取固定 Noto ID。字体身份摘要/cmap 不代表可见 frame fence。

[processed 作者证据](snapshot-evidence/processed-author/README.md)：当前真实78/65 tests、Clippy/build/TS0与LvQxtZ输出均已保存；[独立13PNG验证](snapshot-evidence/independent-processed/review.md)已通过旧快照/独立recipe/raw/SAR/±offset/off及CAS/缺字拒绝；首次分析器PATH ffmpeg缺libass反例原样保留，改用明确full binary后只复算现有产物，没有重跑宿主或改断言。

processed现已接includeSubtitles:false，不需改播放器selection，不用expectedSubtitle；默认产品未勾选时显式传false。该新选项与audio/MP4处于本轮候选独立验收中；此前PNG独立证据仅覆盖selected外部字幕子集，不追认为该选项/原生字幕/完整M3已通过。

## 当前媒体时间域修正（待独立复验）

原native独立run RMpbEu揭示：默认lavf探测将实际容器start_time=2错误报告成demuxer-start-time=0，而播放器position从2开始。原失败保留。当前本地租约load显式采用公开 `demuxer-lavf-probe-info=yes,rebase-start-time=yes`，以成熟后端归零媒体起点；capture固定当前实际load的容器起点，worker以heldFile的ffprobe核对该起点。缺失或不匹配均failed且不发布，不凭mpv字段单独宣称文件域已验证。

snapshot.timeline为 `{domain:"mpv-media",validation:"pending-worker-origin-match",containerStartTime}`；job.facts.sourceTimeline记录实际/expected起点及 `normalized-origin-matched` / `unsupported-file-origin`。native提取采用 `-copyts -start_at_zero`，与视频默认输入归零语义一致；快照不提前声称native内容/cmap已验证。

`duration`是后端元数据估计，并非任意t都有帧的保证。PNG实际无帧失败；MP4完整解码之外另核视频track的实际duration，短于请求区间超过一个编码视频帧间隔加0.1ms则失败，不以音频apad/format.duration掩盖视频不足。该检查在后台执行，不在控制线程扫描来源。作者8J7XUq证明zero/origin2的time1.5 raw/processed带native字幕PNG分别exact一致；修后独立HsHXOs亦完整PASS（待正式报告链接）。origin2真实尾8但metadata10时，[7.5,9)只有0.5s视频，failed且无发布。修后独立HsHXOs已通过同源零/非零起点、真实尾界与active encoder取消，仍不能泛推任意timestamp断层/所有容器。

修后原生字幕/采样/归零时间域/真实active encoder取消的[独立完整报告](snapshot-evidence/independent-native-burst/review.md)已归档，HsHXOs实际PASS；原RMpbEu反例继续保留。所有有限格式/字体/设备与其余M1–M4缺口不因此自动发布。
