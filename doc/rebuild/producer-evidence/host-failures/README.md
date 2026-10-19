# 原共享AV宿主失败，保留不追认

DxvGP0：公共来源普通文件Ready后，候选AV audio callback ack/2048 PCM/video均已齐，但deviceErrors=1，故ready failclosed。首实现无错误详情，不归因。

m1MzFa：增加预分配rtrb有界错误诊断后真实run仍失败，候选active=false、epoch1 paused=true、received0、hasVideofalse时明确收到 `Xrun: A buffer underrun or overrun occurred.`，errorOverflow0。strict smoke在该错误处失败，未继续 AV 激活或称PASS。每run完整素材/consumer/commands/stdio/failure保留同名目录。此CoreAudio overload通知与自身PCM欠载计数underrunFrames不同；不把stream断言死亡，也不清生命周期计数。

下一仅一次基于公开supported范围的Fixed1024buffer配置修正及实际协商facts，增加约一buffer延迟。若有后续通过，仅该新配置/新run有效；原反例不会覆盖或追认为通过。未做声学或设备drain证明。
