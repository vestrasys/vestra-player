# 首activation作者失败

b5qTwg 宿主实际exit0、外层exit1。a真实resolve→Ready、b真实throw→Failed并撤commands、c真实1000ms期限→TimedOut均已走；d初UI headless拒绝实际message是“mounts require a native shell”，脚本/headless|unavailable/断言未匹配，未标总pass。完整原失败/trace/commands/stdio及二进制SHA保留。

Host当时只将UI RpcError.message写入activation失败receipt，丢了原code（unavailable）。下一窄修保code前缀以便管理者诊断，原断言/目标不降低；这不是将headless拒绝冒作真实Slint编译验收。
