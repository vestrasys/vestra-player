# 字幕公共资源候选（D，未发布共同基线）

当前主宿主已有有界上传、严格 SRT 校验、owner 资源、选择 CAS、操作恢复及实际 libmpv 轨道接入；不是产品字幕插件。SDK 为 `src/sdk/subtitles.ts` 的 `host.subtitles`，manifest 能力是 `subtitles.read` / `subtitles.control`。当前仍 `vestra.experimental/0.1`。

## 使用与状态

先查询 `state()` 取得 `source.generation` 及 `selection.stamp={generation,revision}`。用正 safe integer `uploadId` 调用 `begin(id,generation,expectedBytes)`，按字节 offset 连续 `append(id,offset,base64)`；单块最多 64KiB、单文件最多 2MiB。已接收范围同字节重发幂等，异字节/缺口冲突。`uploadState(id)` 可恢复传输进度，`discard(id)` 幂等且不使旧 token 重新可用。

`commit({operationId,uploadId,sha256,resourceId,revision,expected,inputOrigin?})` 在一个后台串行仲裁点检查来源、选择 stamp、可选原文身份及当前 contentRevision，校验完整上传后原子注册并选中。原文资源即使未被选中且 selection stamp 未变，旧 inputOrigin 仍拒绝。`select(target|null,expected,operationId)` 与 `remove(resourceId,expected,operationId)` 使用同一选择 CAS。目标是 `{kind:"embedded",id}` 或 `{kind:"external",owner,resourceId,contentRevision}`；null 是关闭。不得解析 backend Debug 字符串推轨道事实。

返回状态明确 canonical `format:"srt"` 与 SHA256，另有 `derived:{format:"webvtt",digest}`。原始上传字节不可变，派生载体不冒充原文。资源有限排序，owner 停用撤其贡献；首次自有外部选择可保存一份有效 baseline，别的调用者显式重选/关闭会撤掉旧恢复资格。没有递归 undo 栈。读取到的历史 Arc 可保字节，不等于仍可选择已撤资源。

`confirmation` 是 `waiting|applied|uncertain`；applied 表示受控 backend 轨道及 sid 已确认，不是实际屏幕呈现 fence。来源 Loading/Empty/Error 撤有效 native load binding；只有真实关联 FileLoaded 的 entry+ticket 才能绑定轨道 ID。相同 generation 的 restart 也要重新加载载体；旧 ID 不得用于新 load。轨道 `styleSupported` 仅已知文本 codec 为 true，位图/未知 codec 为 false 并带 reason。typed 样式路由见下节；实际呈现与字体身份确认继续分别记录。

## 有界未知结果恢复

commit/select/remove 的 `operationId` 由调用者在发送前固定，实例内正 safe integer 单调递增。同 token、同规范 typed 参数重放原成功或原失败；异参 conflict。`operation(id)` 返回 pending / unknown / unknownExpired / completed(result) / failed(error)。每 owner 128 pending、512 terminal，淘汰不重新执行；owner 结束清本。typed 无效在 admission 前拒绝；admitted stale/unavailable/backend 失败占用 token。

新 admitted 请求先检查来源 expected，不符优先 stale，即使换源已撤上传。已保留的同 token/同参数仍重放历史 receipt，不重新检查当前来源或作用于新来源；历史 receipt 不是当前应用状态。失败 commit 保上传供查询/discard，可能已 complete；成功 commit 回收上传但保其高水位。错误码包括 invalid、stale、conflict、missing、busy、unknown-expired、unavailable、backend、backend-uncertain。实际后台失败的回滚尽力恢复旧 sid 并清受控新轨，恢复失败保 uncertain；尚未注入真实 backend 半失败，不能从协议拒绝保旧推断所有 backend 回滚已验。

## 成熟字幕载体与限制

使用锁定 subtp 0.2.0 公开 SRT parser；UTF-8 可选 BOM、LF/CRLF，最多 10000 cue，递增正序号、非递减起点、start<end、24h 内、单 cue 4096 字符。保留字面正文；不接受位置扩展。派生标准 WebVTT 仅对正文的 &<> 做一次实体编码，brace/backslash 交给成熟 FFmpeg WebVTT decoder 转 libass，避免 raw SRT 的 HTML/ASS 特殊序列被解释。官方路径与 standalone 真实像素证据见 [字幕后端调查](subtitle-evidence/README.md)。`sub-text` 的 strip ASS 诊断不保证原字符串 roundtrip，不能为了它逆转义正确像素。

`sub-auto=no` 防止 mpv 静默加载未登记 sidecar；后续发现策略应由公开插件提供。sub-add 回包不含轨道 ID，宿主按受控 URL 与前后 track-list 关联，失败仅清自己可确认的候选；无法确认/清理即 uncertain 并保载体，不任意删除他方新轨。

尚缺：typed owner 样式/偏移独立应用验收、已验证字体表与字体 lease、1080 参考样式到实时 mpv/离线 ASS 的统一映射、不可变任务呈现快照。`subtitle_style.rs` 已 mod 并实际 all-targets 覆盖；style 路由与 SDK 已接，外部受控字幕的撤层像素修复已通过，仍不据此称任意内嵌 ASS 样式通过。M3 source lease/preroll CLI 是风险实证，尚不是公共 snapshot/jobs。

## 当前验证

作者真实 `node src/tests/subtitle-review.mjs` exit 0；[原始协议证据](subtitle-evidence/author-protocol/result.json)覆盖 >1MiB 分块、重复与冲突、CAS、旧原文 revision、失败保旧、操作重放/查询、换源与多 owner 撤销。最后 all-targets 测试 host 53 / composition-probe 40 全过，Clippy -D warnings、TS、build 均成功；[日志](subtitle-evidence/author-build/vestra-subtitle-final-tests.log)。Empty/Error binding 窄修后 build 0；[独立完整协议与像素结果](subtitle-evidence/independent/review.md)均已通过，含 close→disable。暂停同源实际字幕 on 三个样本各5933变化像素，off四样本精确回基线；代表图实见本例字面 HTML/ASS 样正文。不外推全字体/样式、屏幕 fence 或设备音频。

## Typed owner 样式层（下一有限块）

`style({operationId,revision,expected,style})` 使用同一字幕 operation 本，expected 为当前 source+selection stamp，样式 revision 正 safe integer 递增。`style:null` 撤当前 owner 层。每 owner 一层、最多32层，以 manifest ID 固定字典序最后者为有效层，不以加载/调用时间改变优先级；停用只撤自身层。更低优先级更新保资源记录但不覆盖较高层。当前只对 Ready、已确认且支持文本样式的选轨应用；无媒体默认由 TS 保留。typed invalid/未知 font 在 admission 前拒绝，后端失败恢复旧 property 集合，恢复失败明确 uncertain。

样式 `{fontId,fontSize,color,outline,marginPercent,offsetMs}`：整数16..72、white/yellow/cyan、描边0..4、边距2..15%、偏移±10000ms。1080高视频参考到 mpv720参考：字号及描边×2/3，margin×720/100，正offset秒延后；强制底部居中/黑描边/无shadow、按视频而非窗口参考。颜色依据 mpv 实际 #AARRGGBB，不误用 RGBA。该映射没有冒充 offline libass 的默认 PlayRes288坐标。

`state.presentation` 显示 layers/effective/confirmation、referenceHeight1080 与有限 fonts。当前本机系统 Arial Unicode MS 请求 family 有文件可用，但 `actualFont:null,snapshotEligible:false` 明示尚无精确字体身份/不可变字体 lease；不能用请求family或getter字符串发布可复现导出快照。字体不存在时不猜成功。这里未分发系统字体。

首轮所有 mpv baseline properties 已恢复却暂停字形变空，原失败保 [style-revoke-failure](subtitle-evidence/style-revoke-failure/README.md)。修复仅 override mode 切换时，对本次 load 实际绑定的宿主登记外部载体执行公开 sub-reload，重新核受控URL→track并确认sid；常规同mode连续参数不reload。作者真实 [七窗样本](subtitle-evidence/style-author/README.md) 已通过小白/大黄/正偏移隐藏/回0/撤层精确原默认，source/pause/position保持。[独立main/replay/内嵌撤层](subtitle-evidence/independent-style/review.md)四实例均pass：双owner、samegenrestart、新B内嵌Ready无新字幕mutation重放，以及B撤层下60%ROI与独立默认对照exact0。不据此声称任意ASS/位图、全字形字体或离线parity。
