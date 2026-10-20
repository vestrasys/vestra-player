# 播放信息（vestra.info）

Tab 或「视图 → 播放信息」开关视频左上角的只读信息层（默认关），每秒刷新：来源、时间 / 时长、视频编码与分辨率与帧率、音频编码 / 采样率 / 声道、后端、会话、已显示帧数、丢帧（原宿主诊断行的全部内容）。

用到的接口：`plugin.osd.panel()`（能力 `ui.osd`）、`plugin.player.stats()`（`playback.read`）、命令 `bindings`。外观细节（UI-INFO-02）在第二步调整。
