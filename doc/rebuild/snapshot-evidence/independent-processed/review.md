# Processed PNG 独立有限复核

2026-10-17，独立审查者 `/root/rebuild_bootstrap_review`。root/maintainer 明确唯一最终冻结后，独立执行 `node src/tests/media-processed-review.mjs --run`。实际宿主 exit 0 / signal null，公共消费者原预期及 13 个 PNG job 均完成。没有构建、GUI、改 shared/API 或重复旧 raw/rotation 套件。

第一次外层 driver exit 1：PATH 的 FFmpeg 不含 subtitles filter，独立 old recipe 参考生成报 `No such filter: 'subtitles'`，不是当时已证实的宿主失败，也未将当时结果称为独立通过。原始 [reference-tool-failure-first.json](run/reference-tool-failure-first.json) 与 pixel-failure.json 保留。root 同意后，只修 owned 分析工具显式使用 `/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg`，执行 `--analyze-work` 复用同一批原始产物；analysis-only 实际 exit 0、全部原像素预期通过，没有重跑宿主或降低断言。详见 [independent-execution.json](run/independent-execution.json)。分析结束后已明确交还冻结。

原 work：`/var/folders/c9/l6fw8b9n69d_4030n4dcj2nh0000gn/T/vestra-media-processed-review-BagMMp`。完整 [result.json](run/result.json)、[consumer 原始事实](run/consumer-result.json)、host command/stdio、执行 consumer/helper、13 张实际 PNG、独立 reference PNG/VTT/SRT/固定字体 bytes 与 LICENSE 均已持久保存。测试视频未复制，生成方法在新独立脚本；字体原 asset 亦在新 src/assets。没有冒称原生截图或未保存的独立消费者 stdio。

## 旧快照实际保留的内容

自有 640×360 / SAR2 静态网格 A 红角标、B 蓝角标。helper owner 2 贡献 eq brightness=.12→gblur sigma=1.3、外部 SRT `旧字幕 <b>字面</b> {\b1} \N OLD`、黄色固定 Noto、fontSize72/outline1/margin5/offset+250ms。main 在 A 上 capture processed time=1.5，冻结后先取得 [old-before.png](run/artifacts/old-before.png)。

捕获摘要经实际字节与期望核对：sourceStamp gen1/intent1；processing revision=1，包含 helper 的 eq/gblur typed 配置；subtitle selection gen1/revision1、presentationRevision=1；canonical SRT SHA-256=`e273a6722cc4987131939db1db65002840ac4447589bcfc7e4a80875db0d59a3`、derived VTT SHA-256=`7b2b6ffdd01fec8ef43993d46ca262c525b301774b8c8b7a0dc2b42e6bfebd7b`，均与独立准备的实际 bytes 一致。font ID 为 noto-cjk-sc-regular-2.004、faceIndex0、16,437,364 bytes、digest=`2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`，与实际固定字体文件 hash 一致；coverageValidated=true。presentation=1280×360。

随后真正停 helper，公开其效果资源/字幕资源均撤去；main 改为 -.1 brightness、新中文字幕、青色 style，并取得新 A 产物；再切 B/提交新字幕/style。始终不重 capture 旧 snapshot，而从它再次取得 [old-after-source-owner-changes.png](run/artifacts/old-after-source-owner-changes.png)。

| 实际 PNG 比较 | 全 RGB 结果 |
| --- | --- |
| 旧 snapshot 前→停 owner/换配置字幕style/切源后 | MAE 0 / exactDiffChannels 0 |
| 旧 PNG→独立 old recipe | MAE 0 / exactDiffChannels 0 |
| 旧 PNG→新 A 配置 PNG | MAE 63.21906032986111，实际明显不同 |

独立 old recipe 从保留 A 副本、自己写出的字面 VTT、相同固定字体 bytes 出发，显式 eq→storage gblur→SAR→shifted subtitle clock→trim1.5，不导入宿主 mapper。实际 old 图下半区有 652 个黄色 pixels；独立查看可读本例中文字幕和字面标签/ASS-looking 内容。额外将黄色 glyph bbox (517,323)…(763,339) 单独对参考量化，ROI MAE=0 / exactDiffChannels=0，见 [glyph-region-analysis.json](run/glyph-region-analysis.json)，没有只以全帧平均误差低推断字幕存在。

这是本例有限 glyph-region 像素等效与固定资产摘要证据，不是 FFmpeg/libass 字体 provider 排他、实际 face 选择的独立证明。reference 与产物同用成熟 FFmpeg/libass，不能排除共同实现或共同字体替代行为；本轮也只有一个公共可用字体，未测试动态替换另一 font asset。cmap/字体 hash 自身不等于任意文字 shaping/视觉字体身份保证。

## CAS、raw、cue offset/off 与 SAR 顺序

三个独立 capture 条件分别用旧 processing revision、旧 selection stamp、旧 presentation revision，均返回 admitted stale，operation 查询保存对应 failed 结果；保留旧 snapshot.state 完全不变。不是实际 backend 半失败回滚试验。

当前 B 有新效果和已选新字幕，raw PNG [raw-b-unbaked.png](run/artifacts/raw-b-unbaked.png) 与独立无效果/无字幕 B reference 全 RGB MAE=0 / exactDiffChannels=0，未被 processed 配置污染。

字幕 cue 是 [1s,2s]，采样避开帧量化的精确边缘：

| 公开处理配置与 snapshot.time | 实际结果 |
| --- | --- |
| offset0，t=.5 / 2.1 | 无字幕，精确匹配显式 off 图 |
| offset0，t=1.5 | 449 个青色字幕 pixels |
| select(null)，t=1.5 | 冻结 subtitle=null，0 青色 pixels |
| offset+500ms，t=1.4 / 1.6 | 前者精确匹配 off；后者精确匹配原 on 图 |
| offset-500ms，t=.6 / 1.6 | 前者精确匹配原 on；后者精确匹配 off |

每项实际 PNG 保留在 run/artifacts/，对应 exact 通道差全为 0，见 result.comparisons.off/on。没有把活跃 source 的 paused time=0 当离线字幕评估时钟。最后仍 source gen2/intent2、paused=true、position0，离线时间处理未显式 seek/unpause 宿主。

真实 U+10FFFF（非字面反斜杠文本）的外部字幕被选择后，processed capture 返回 unsupported：固定字体缺少 U+10FFFF glyph；operation 记录 failed。没有静默把缺字当完整支持。

字幕 off 后，仅安装 sigma2/steps3 gblur：实际 [b-storage-blur-before-sar.png](run/artifacts/b-storage-blur-before-sar.png) 对独立 storage blur→SAR 参考全 RGB MAE=0 / exactDiffChannels=0；对故意错误的 SAR→blur 参考 MAE=6.880991753472222、1,328,155 差通道，fixture 实际辨别了顺序。reference 保存于 run/references/，未沿用旧 scale-first 假设。

## 有限结论

本轮 processed PNG 子集的旧 source/effects/text/style/固定 font 资产摘要、贡献 owner 撤销后保留、三类 CAS、raw 不烘焙、正负 offset/off/缺字、SAR2 storage effects→SAR，获得独立公共消费者及实际 PNG/明确 reference 证据，有限通过。首次工具失败完整保留，分析修复无宿主重跑。没有本轮新增阻断项，可继续必要 M3 集成。

此结论不是 native 字幕、任意字体/provider 排他、任意角度/色彩管理、MP4、音频处理/preroll/设备声音或完整 M3 通过。audioOffsetMs=0、audio=[] 是冻结描述，不是声音效果测量。未测任意 backend 半失败、全 scheduler interleaving 或抗恶意原地文件改写；也没有 native PTS/来源原子 frame fence 或物理屏幕证明。
