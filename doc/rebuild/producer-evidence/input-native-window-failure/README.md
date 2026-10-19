# 首原生input probe：窗口未能绑定，未验收

唯一bundle ID local.vestra.rebuild.producer-input-d9a9d6c51e06/PID15094，完整args及ready.json成立；root独占CUA inventory见running，但按ID与精确app路径getApp均cgWindowNotFound，displayName Invalid；无任何键鼠输入。不能把Node READY/脚本停止成功当真实原生输入通过。

root通过自有stop文件收尾，result outcome=stopped/failure=null，source.close completed，appliedReceipts=0。记录一次active Xrun和大量软件underrun，核心约10fps，不能声称原生AV连续。本probe每帧3次串行RPC经UI timer往返，下一author-only修正减少轮询并有界批量预填；这只是定位依据，未称已修。

own-process sample显示主线程在Slint/winit event loop，并非字体/文件read挂起。没有使用CG/osascript/额外GUI注入。source正常清理后明确SIGTERM精确原PID释放不可绑定窗口，termination.json记录。执行脚本及全部原stage日志/结果/配置/依赖素材在run/（大型bundle二进制未复制，其SHA在../input-build/identities.json；当前源码可重建）。300秒窗口是准备后显式有限时长覆盖，launch.json保变更原因；不是原脚本120秒默认值。

下一轮先加现有public window.state/native关联事实，仍保本反例。
