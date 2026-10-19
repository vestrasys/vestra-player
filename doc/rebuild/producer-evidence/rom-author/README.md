# 真实 Thwaite / JSNES 到宿主 AV 作者纵切

zJ53XJ 实际宿主和作者脚本均 exit0；使用同一已验证控制握手二进制，没有重新 Cargo 构建。输入为固定官方 JSNES2.1.0 与合法 Thwaite ROM，SHA及许可在 run/。依赖先校验复制并安全解到本次新临时目录，再开启设备。不是产品插件或独立验收。

真实核心完成两次 Start 的540帧暖机进入既定游戏场景，产生431301个stereo样本回调、103711个非零音频帧；RGBA按官方实现低字节R/G/B转换。随后逐NES.frame采集真实PCM与视频，每次写入await，基于该帧实际样本数建立时间映射，生成领先消费上限4096样本。核心帧、样本时钟及宿主videoFrames均在两段约700ms feed中前进。

在NES.frame调用之间停止执行、releaseAll后处理公共Notice并echo；原操作等待宿主callback确认后完成。pause与220ms held的consumedFrames均34816，真实核心帧和生成样本游标保持；恢复后消费至67584，核心最终632帧/504782样本。source.close后直接观察旧producer不可用，未先release掩盖自动回收。

本轮deviceErrors=0，但恢复时软件underrunFrames已有1024、后续2048；软件欠载写零不推进媒体时钟。不能称无声学间断、硬件时钟或低延迟验收。暂停丢弃有界预生成PCM，恢复把新核心PCM映射到Notice.baseSample；无checkpoint回退或零丢样承诺。无真实屏幕像素、键盘焦点、save/load/reset/newROM公共路径或十分钟稳定性。所有结果、core/输入/映射/health观察、ROM、精确依赖及许可、命令、stdio均在run/。
