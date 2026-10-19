# NES 探针 API 请求（实验，非最终契约）

## API006：外部来源暂停状态

- 请求：`sources.setPaused({key, paused})`。模拟器自己停止执行帧，但公共播放状态也必须反映暂停；不能借用视频暂停来伪装来源时钟。
- Maintainer：已批准并实现 SDK / Rust。来源保持 `duration:null,path:null`，按 owner 和 key 校验。
- 使用：暂停先等待正在提交的帧结束，再提交状态；恢复先提交状态再继续独立 60Hz 调度。
- 输入：公共 `onInput`；开发宿主 `host.input` 进入真实 worker，GUI Slint 按键由通用宿主桥接。

## 实现反馈：PCM 消费与诊断

真实端到端测试发现 ffplay 9 已移除 `-ac` 输入选项，造成来源音频管道关闭。已报告 maintainer，需使用 `-ch_layout stereo`；同时请求帧提交在音频队列失败时不得半提交，并保持有界背压策略。此问题不是模拟器自报成功能够发现的。
