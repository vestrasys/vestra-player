# 截图与片段导出 API 请求（实验）

本目录遵循公共实验契约，最终产品 API 可能相差很大。

## API-004：输出对话框与字幕采样

- 操作：用户选择尚不存在的 PNG/MP4 输出文件；处理后截图可独立排除字幕。
- 原因：`openFile` 只选择输入，`processing: 'current'` 原先不能排除字幕。
- 提议：`dialogs.saveFile({title, extensions, suggestedName})`；`media.start({processing:'current',includeSubtitles:false,...})`。
- 所有权：输出归用户选择；任务与临时文件归宿主的调用插件 owner。取消选择不启动任务；任务取消清理未完成输出，已有文件不得覆盖。
- 最小验收：真实保存有效 PNG；比较原始、处理后及字幕开关像素；取消任务不遗留半成品。
- 维护者 `/root/api_maintainer` 已批准，记录为共享 API-004，默认包含字幕以保持兼容。

## API-005：处理链快照句柄

- 连续截图是多个有限子任务，逐次使用 `processing:'current'` 会混入媒体切换后的处理链。
- 原提议使用公开 `effects.list()` 拼接表达式；维护者拒绝该方案，因为插件不应复制宿主处理链组合规则，也会遗漏 legacy Gaussian 等路径。
- 维护者批准：`effects.snapshot({includeSubtitles}) -> {id}`；`media.start({processingSnapshot:id,...})`；`effects.releaseSnapshot(id)`。
- 句柄由 owner 隔离、内容不可变；任务启动复制处理参数，组任务在 finally 释放句柄；卸载回收所有快照。
- 最小验收：开始连续截图/片段任务后切换媒体及处理参数，任务仍使用原输入及原处理链。
