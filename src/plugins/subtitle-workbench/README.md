# 字幕工作台探针（实验）

本插件已按公共候选契约 probe-0.3（API-007）完成调用迁移与作者回归，不代表最终 API。只依赖本目录与公共 SDK，不依赖其他探针；迁移与回归状态见 [API 请求](API_REQUESTS.md)。

## 使用和策略

先打开视频，再从“字幕 → 来源 → 加载 SRT 字幕”选择文件。原生 Slint 面板支持显示/隐藏、字号、白/黄颜色、±5 秒滑块同步（API 校验范围 ±60 秒）、原文/译文轨道选择。下一条字幕会 seek 到相应 cue。支持 BOM、多行文本、多条有序 cue；拒绝无效时间。

文件名可有引号、空格与方括号：读取后写入插件独占临时目录，通过 `files:{captions:cache}` 声明资源，filter 仅引用 `{{file:captions}}`。宿主复制并管理内容，供效果、快照与任务持有，不把用户路径拼入 FFmpeg filter，更不构造 shell 命令。字幕效果通过 `effects.set({kind:'subtitle',sessionId,...})` 交给真实 Rust/FFmpeg，字体样式使用 libass。必须安装带 `subtitles` filter 的 FFmpeg；未具备该编译特性的发行包不能合成字幕。

翻译通过可替换 `TranslationProvider`；默认是 300ms 的确定性异步 mock，仅模拟外部服务，实际字幕解析、SRT 写入、宿主合成都保留。`activateWithProvider` 可用于将来的网络提供者，不要求 BYOK。取消由 AbortSignal 传递；revision 和最终宿主 snapshot 双重阻止旧会话结果。任务不阻塞 runtime 事件队列。失败保留原文，可重试。

切换媒体清除当前字幕及译文，取消翻译，要求重新加载字幕；字体/颜色/同步偏好保留。隐藏 UI 不停止处理；重建 UI 使用宿主缓存属性及原 TS 进程恢复业务状态。卸载移除自己的 captions 效果、取消任务、删除临时文件并解除订阅。

## 验证

```sh
src/node_modules/.bin/tsc -p src/tsconfig.json
.cache/probe-target/debug/vestra-slint-experimental1 --check-ui src/plugins/subtitle-workbench/manifest.json
PATH=/opt/homebrew/opt/ffmpeg-full/bin:$PATH VESTRA_HOST="$PWD/.cache/probe-target/debug/vestra-slint-experimental1" node --test src/tests/probes/subtitle-workbench/subtitles.test.ts
```

- S1：真实 worker 选择包含特殊字符的 SRT 路径，解析两条时间轴；真实宿主输出 PNG 并解码 RGB，证明字幕像素、字号、颜色、同步和隐藏的差异。不是只断言 effects RPC。
- S2：mock 翻译期间切换媒体，无旧效果/译文；错误后重试、取消、原译轨道切换、卸载清理。
- strict TS 与单插件 Slint 编译已通过。首次检查揭露普通 FFmpeg 未编译 libass；安装 `ffmpeg-full` 并修复 seek 后字幕绝对时间后，root 重跑三项测试通过，包含真实字幕像素。运行命令需将 `/opt/homebrew/opt/ffmpeg-full/bin` 加入 PATH（或使用项目环境脚本）。
- 独立验收发现原测试未覆盖卸载临时目录清理，以及导出快照引用字幕文件后的内容冻结问题；维护者修复与独立重测记录见[审查报告](../../../doc/probe-review.md)。此前测试通过不代表这两项已验收。
- G1 真实窗口加载、菜单可发现性由协调/独立验收层另行执行，静态 Slint 编译不代替实际窗口证据。

## API-007 作者复核

原作者依据工作流 3.1/3.2 复核了协调者的窄迁移：`subtitleFilter` 只生成具名占位符，`files` 显式声明由宿主复制的资源；字幕解析、翻译、菜单和生命周期策略未变，也没有让插件读取宿主缓存路径或导出插件目录。原有真实像素、第二 cue 绝对时间、样式、同步和会话断言保留。

在 probe-0.3 当前宿主上重新执行：strict TypeScript 退出 0、单插件 Slint 编译退出 0、三项字幕测试全部通过。另在真实处理测试中增加两条消费者回归：字幕插件改写自己的 SRT 缓存后，旧快照仍输出完全相同的原字幕 RGB；字幕插件卸载并删除自己的缓存后，另一 owner 的旧快照仍可成功合成相同 RGB。测试随后释放快照，不改变最终清理预期。

成功产物位于 `.cache/probe-evidence/subtitle-workbench/`，含原始/字幕/第二 cue/大字号/黄色/延迟 PNG，以及 `frozen-after-update.png`、`frozen-after-unload.png` 和 JSON 记录。以上是本插件作者回归，不替代独立验收者对宿主缓存最终引用清理、组合场景和 G1 真窗口的验收；probe-0.3 仍为候选基线。
