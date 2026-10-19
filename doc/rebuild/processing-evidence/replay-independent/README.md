# 切源重放有限独立像素证据

独立审查者在冻结构建运行`node src/tests/processing-replay-review.mjs`，宿主与测试退出0。root直接读取result/command并归档，未更改原数据。作者四窗口结果另在`../replay-author/`，不混为同一运行。

| 稳定采样窗口 | 中心RGB均值约值 | 实际来源角标 |
| --- | --- | --- |
| A基线 | 128.082 | 红 |
| A运行时亮度0.2 | 186.301 | 红 |
| B暂停，未再replace | 186.301 | 蓝 |
| B带norm链，暂停亮度0.3 | 216.575 | 蓝 |
| B同generation重播，未再replace | 216.575 | 蓝 |

每窗口3张PPM逐张通过脚本阈值。B暂停两窗口position0、generation2/intent2保持；重播同generation2，cycle2→3、entry2→3。角标实际像素用于区分来源，未拿当前generation贴到缓存帧当作来源栅栏。

`pixels-and-fixtures.tar.gz`包含**全部**诊断`frames/`（PPM及frames.jsonl）、新生成的`a.mkv`和`b.mkv`，无抽样删减。可用`tar -xzf pixels-and-fixtures.tar.gz -C <空目录>`展开；其余JSON/stdio在本目录直接可读。JSON保留原运行临时路径作出处，不伪装展开目录就是原环境。重新执行脚本会生成新素材/新日志，不依赖旧路径。

范围限实际libmpv软件renderer像素及来源/处理记录。诊断显式`frameSourceAssociation: none`，不是PTS/首帧fence或物理显示采样；等待稳定后的正确像素不承诺切源零过渡。norm节点在真实音轨上安装，但未采集PCM/设备音频，不能推断音频处理效果、无断音或DSP历史完整保留。故障回滚、音频延迟和八插件完整交互仍未通过。
