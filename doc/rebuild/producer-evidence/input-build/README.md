# 原生 physical input 候选构建

首check失败为composition-probe未引新增shared模块，修后alltargets check2 exit0（53.99s）；fresh TS首轮因SourceTicket接口不满足Json索引签名失败，改等价type alias后exit0，无any绕过。日志保留。

当前全targets测试：composition-probe106、host124，全部0fail，含真实Compiler构建共享壳/两组件（不创建窗口）、输入State5/Delivery5和native queue满keyup→pinned空集的adapter回归。Clippy -Dwarnings exit0（3.41s），feature host build exit0（6.42s）。未以这些结果代替真实原生键盘/焦点/release ACK验收。

锁Slint1.16.1并使用公开safe但明确unstable-winit-030适配；Winit0.30.13 physical code，不从text猜。Rust只有限physical集合，不映射NES逻辑按钮。主线程只转内部带scope事件，仲裁线程执行声明/完整Notice FIFO/ACK与周期expiry。普通full key事件被拒时撤为TransportFailure empty barrier，不能静默丢keyup。pending revoke未ACK前不得capture/key覆盖，失败超时停来源消费。

identities.json绑定本次二进制和相关源/SDK，正式native probe启动前须一致。
