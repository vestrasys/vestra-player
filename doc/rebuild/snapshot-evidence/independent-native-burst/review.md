# Native 字幕、有限连拍及活跃编码取消：独立有限通过

2026-10-17，修后唯一真实消费者 HsHXOs 与独立 FFmpeg 分析均 exit 0。本轮在明确 source/SDK/binary 最终冻结后执行；未构建、未运行 GUI、未重复旧 PNG/MP4 套件。host 退出后已即时交还冻结。结论仅覆盖下述实际素材与有限路径，不代表完整 M3、物理音频或所有取消竞争通过。

## 原始材料

- `preparation/`：自有 1280×720、30fps、8 秒红 A/蓝 B 运动素材、三个 native 中文/英文 SRT cue、markup/ASS/origin2 变体；实际 ffprobe、独立提取与固定字体、不同时间参考 PNG/MP4、全部工具命令。相邻五参考图非相同静帧。
- `origin-failure/`、`origin-failure-review.md`、`executed-review.mjs`：首轮 RMpbEu 完整失败及未改原断言。非零文件 origin 被成功 capture，而当时未证明归一化；该反例保留，不追认通过。
- `normalized-pass/`：唯一修后 HsHXOs 的完整实际 fixture、已执行插件/helper、14 个已发布产物（13 PNG、1 MP4）、消费者 JSON、host stdio、command、逐工具命令/诊断、独立解码结果和 `independent-summary.json`。`normalized-review.mjs` 固定修后消费者。

受控契约迁移基于本轮 `source-evidence/origin-investigation` 与维护者修正：capture 保存 mpv-media 域及实际文件 origin，标记 pending-worker-origin-match；worker 用 held fd 独立核对 origin，并统一 native 时间轴。原 origin2 同步拒绝断言被更强的归一化像素正向及视频尾部失败检查取代，其余 immutable、样本、阈值、真实子进程判断未删改。仅允许 capture 成功不能通过新断言。

## 实际画面、采样及 immutable 输入

旧 snapshot 绑定真实 selected native subrip stream 1、租约、yellow Noto 样式、+250ms 字幕 offset 与 typed eq brightness .05。capture 的 native coverageValidated=false；每项成功 worker 返回 canonical SRT/derived VTT digest，与独立提取参考一致，cueCount=3、coverageValidated=true。原 snapshot state 仍与 capture exact deepEqual，没有被后续 worker proof 改写。

capture 后撤 helper、改变当前图像处理/样式、开蓝 B 并关闭当前字幕，随后旧 snapshot 的 .5、1、1.5、2、2.5 秒五图，以及 3、3.5、4 秒部分连拍三图，仍分别匹配自有红 A 对应时刻/原 cue/原样式参考。13 张最终 PNG 每个 RGB 通道均 exact，相应 MAE/maxAbsDiff/exactDiffChannels 全为 0；不是仅检查文件数量、请求时间或全图低平均误差。实际查看 `outputs/origin-native.png` 可见红 A 测试图及黄色“原生第一句 NATIVE ONE”。

旧 native MP4 [.5,2.5) 实际 H264/yuv420p、1280×720、30fps、60 帧、2 秒、无音轨，完整解码 RGB 与独立旧 recipe MP4 reference exact。raw 与 includeSubtitles:false 的 PNG 各匹配自己的独立参考，排除字幕；raw 同时排除 typed 图像效果。

## 实际非零文件 origin 与视频尾部

origin2 容器/流实际 start_time=2，已在独立 preparation ffprobe 核实。修后公开 Ready position=0；explicit seek0 后仍 0、paused=true、generation/cycle=5 保持，intentVersion 按控制操作从5增至6。三个 snapshot 实际冻结 containerStartTime=2；worker sourceTimeline 实际值及 expected 值均2，validation=normalized-origin-matched。native/raw/exclude 的1.5秒 PNG 分别与 plain0 自有对应参考全 RGB exact，native 规范化两 digest 也与 plain0 相同。因此本素材实际归零及相对字幕时序已证，不是从 generation 标签推 frame 关联。

公开 metadata duration 仍为10，而独立素材真实归零视频尾为8。本轮请求 PNG8.5，worker failed/export-failed（无有效 PNG），publicationStarted=false、无最终文件。raw/processed MP4 [7.5,9) 均实际生成视频仅.5秒，worker output stream facts duration=.500000，随后明确 source-unavailable/视频短于请求1.5秒拒绝发布；publicationStarted=false、无最终文件。该素材无音轨，本轮没有实际测试“音频填满而视频不足”变体，也没有证明任意时间戳跳变、未知或负 origin。

## 部分停止、真实活跃编码取消与边界

TS caller count2..20/interval.1..10、末采样点严格小于 duration 的本地策略边界有断言；这不是新增 host 连拍 API。部分序列先实际发布三图，再观察 host5819 真实 PNG encoder5941（/dev/fd/0、-c:v png）运行才请求取消第四项。requested→cancelled，未发布，取消后同宿主匹配 encoder 列表为空；第五项从未提交，前三张保留且像素仍 exact。

补足前轮 MP4 queued-only 缺口：UTC 2026-10-17T21:34:03.008 观察 host5819 子进程6043，真实命令含 /dev/fd/0、-filter_complex、-c:v libx264、目标8秒 MP4；公开 job20 为 running 且 publicationStarted=false。在同一毫秒记录公共 cancel 返回 requested，随后 terminal cancelled/未发布，匹配 child 不再列出，最终无目标文件或 .tmp。原始 PID/父 PID/完整命令/墙钟/轮询/公共状态均保留在 consumer-result 与 summary。此证据证明真实 encoder 活跃后取消及该宿主下子进程消失；未通过独立 waitpid 观察系统回收状态，不冒称所有 validation/publishing 竞争均已测试。

sampleTime==duration 同步 invalid、operation failed；markup 在 worker 明确 unsupported/export-failed 且不发布；ASS capture 明确 unsupported。legacy brightness 和额外 sub-delay 偏离 typed 值时拒绝 processed，而合法 owner 音频 delay-only 不被误拒。最终源 paused/position0，输出目录只保留14个成功文件，所有匹配 PNG/MP4 encoder child 列表为空。

## 证据范围

采用自有成熟 FFmpeg 参考、真实最终像素与完整视频解码，固定字体 asset/digest、有限字形可见及输出等效已核；不由此证明 libass 字体 provider 排他性。native 仅本轮 plain SRT 子集，不覆盖 ASS/bitmap/所有字幕格式。未测设备声学、实时 DSP 历史一致、所有 owner-stop/发布竞态，未将无音轨 MP4 当音频验收。原 native origin 失败完整保留，当前有限通过不取消其它 M3 发布门槛。
