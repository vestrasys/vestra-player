# 自动右角旋转独立有限复核

2026-10-17，独立审查者 `/root/rebuild_bootstrap_review`。维护者明确自动 geometry 最终冻结后，独立运行 `node src/tests/rotation-review.mjs --run`，两个顺序 headless 实例 main/tail 均正常 exit 0 / signal null，全部原断言通过。未构建、GUI、使用固定旋转 environment probe、改变 960×540 render 目标比例或重跑旧 jobs/样式套件；结束后已明确交还冻结。

原 work：`/var/folders/c9/l6fw8b9n69d_4030n4dcj2nh0000gn/T/vestra-rotation-review-YtSdxK`。

[总结果](run/result.json)、[main 量化](run/main/result.json)、[tail 量化](run/tail/result.json) 与各自 command.json、consumer-result.json、完整 stdout/stderr、执行 consumer/helper 包、全部原始 PPM/frames.jsonl/参考 PPM、实际 raw PNG、各窗代表 PNG 已持久保存于 run/。视频未复制，生成方法在独立新脚本；真实 metadata JSON 已保存。消费者日志经 runner 汇入 host stderr，不冒称另有独立 stdio 或原生屏幕截图。

此前真实 SIGABRT 证据 [rotation-host-crash](../raw-author/rotation-host-crash/) 保留，本轮成功未覆盖/删除它。此次是新四角场景的独立修后复核，不仅仅是原作者红色静态 smoke 或“不再崩溃”。

## 实际软件画面方向与比例

本轮生成四象限 TL/TR/BL/BR 分别红/绿/蓝/黄的 320×180 MP4。真实 display_rotation 0/+90/-90/180 与 SAR=2 的 metadata 经 ffprobe 确认并保存。独立 FFmpeg 默认 autorotate，再 square-pixel/fit 960×540/pad 黑边，生成参考；没有读取后端候选方向当期望值。

实际软件 PPM 每窗至少两个 settled 样本，与参考核四角实际颜色类别、active bbox 各边误差≤3px、预期黑边采样最大通道值<12。没有硬要求不同 renderer/缩放器逐像素一致，也没有仅用尺寸判断方向。

| 场景 | 实际四角 TL/TR/BL/BR | 实际 active bbox（含端点） |
| --- | --- | --- |
| 普通 0° | 红/绿/蓝/黄 | (0,0)…(959,539) |
| +90° | 绿/黄/红/蓝 | (328,0)…(630,539) |
| -90° | 蓝/红/黄/绿 | (328,0)…(630,539) |
| 180° | 黄/蓝/绿/红 | (0,0)…(959,539) |
| +90°、SAR2 | 绿/黄/红/蓝 | (404,0)…(554,539) |
| 0°、SAR2 | 红/绿/蓝/黄 | (0,135)…(959,404) |

有边框场景分别检查 864 / 1080 / 624 个黑边采样点，实际最大通道值均为 0。独立查看代表 PNG，与四角及比例量化一致：[+90°](run/main/r90.png)、[-90°](run/main/rn90.png)、[SAR2+90°](run/tail/sar90.png)、[SAR2普通](run/tail/sar0.png)。

main 按普通→90°→同 generation restart→两次 typed eq 更新→第二 owner 效果层→停该 owner→普通→反向连续走同一宿主。90° 的 generation=2；restart cycle 2→3 后依公开契约只发一次 pause，位置稳定约 0.066667s。随后两次 brightness .03/.05 与 helper 的独立效果层/停用期间，source stamp=gen2/intent4、cycle3、暂停位置保持，实际四角与 bbox 始终仍为正确 90°；helper 公共资源也实际撤去。之后普通和反向均显示各自正确方向。此处验证的是 generic geometry 没被效果图更新/owner 撤销丢失，不另推精确 brightness 量值或全部 DSP 历史。

## 实际 raw PNG 方向与 SAR

每个 fixture 通过公开 snapshot.capture(time=1)/jobs.start rawPng 得到实际 PNG，release 已接 snapshot 后等待 job completed。独立成熟解码器完整解码 PNG，再与原 fixture 的 FFmpeg 默认 autorotate/square-pixel 参考比较。

| fixture | 实际 PNG 尺寸 | 全 RGB 对参考 |
| --- | --- | --- |
| 0° | 320×180 | MAE 0 / exactDiffChannels 0 |
| +90° | 180×320 | MAE 0 / exactDiffChannels 0 |
| -90° | 180×320 | MAE 0 / exactDiffChannels 0 |
| 180° | 320×180 | MAE 0 / exactDiffChannels 0 |
| +90°、SAR2 | 90×320 | MAE 0 / exactDiffChannels 0 |
| 0°、SAR2 | 640×180 | MAE 0 / exactDiffChannels 0 |

实际 raw PNG 保存于 [main/artifacts](run/main/artifacts/) 与 [tail/artifacts](run/tail/artifacts/)，对应参考与 metadata 在 run/references/、run/*-metadata.json。方向由四角内容及独立参考共同支持，不是只看到 portrait 尺寸就称正确。

## 有限结论与边界

本轮冻结的自动 geometry 对显式右角 metadata、两项 SAR2 fixture、同 generation restart、旋转→普通→反向切换，以及 typed 效果更新/第二 owner 停用共存，具有独立实际软件 RGB 与 raw PNG 证据，有限通过，没有本轮新增阻断项。可继续后续必要集成，不代表任意角度、所有容器/解码器或整个 M3 发布通过。

仅覆盖 0/+90/-90/180，未调用非右角来证明其错误路由；候选支持范围不能写为全部角度。静态四象限没有帧精确 PTS/时间变化验证，raw 固定时刻的时间辨识由此前 jobs 专项独立证据支持。本轮未复测首帧发布时刻/加载瞬态，不能把 settled 正确扩为可见 first-frame fence。

PPM frameSourceAssociation 始终 none，observedSource 是观察快照，重复 softwareSequence 可以是 cached settled output。没有物理显示、设备音频、任意滤镜拓扑、后端半失败或 processed export 证明。restart 有意短暂开始播放后再 pause，不声称跨 restart 全程暂停。软件颜色比较允许合理效果变化；raw 全 RGB 精确结果只适用于本轮实际 PNG/reference 对。
