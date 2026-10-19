# 作者样式撤层真实反例

初版 typed 样式 SDK 在暂停灰图上小字号/大黄色/正延迟隐藏及回0生效，但撤层返回成功后画面变灰，未恢复原默认字幕（原默认2500变化像素）。first 与 diagnostic 压缩包保留完整样本、命令、fixture、stdout/stderr和失败断言；diagnostic含实际公开property前后值，均没有 seek/unpause。当前不能称撤层通过。

同时本轮 npm check 的旧 node_modules/undici-types/connector.d.ts read 阻塞，sample保留。同package+lock在/tmp新npm ci--ignore-scripts成功，以fresh TypeScript/typeRoots检查发现 SDK interface不能作为Json参数的TS2345；SubtitleStamp/SubtitleStyle改type alias后tscheck2 exit0。不以此前被阻塞的check声称新增SDK已类型通过。
