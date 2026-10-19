# 当前公共API usage与剩余缺口

已由packages maintainer确认：canonical菜单领域插件、完整leaf路径；普通Window顶级property/callback；能力commands/plugins.manage/ui.window/ui.dialog；operationId严格等于requestId且跨重启同ID精确参数幂等。工坊按这些事实实现。

调用链：用户生成→真实Pi工具写入→真实strictTS诊断修复→packages.validate(directory)→用户trial(token,stableRequestId)→首答复丢失trialStateByRequest(stableRequestId)→trialState(id)→用户stopTrial(id)并等cleanupConfirmed→installed(id).revision→用户install(token,expectedRevision,requestId)→operation(requestId)终态；用户rollback(id,revision,requestId)；取消调用cancel(operationId)。没有直接load冒充安装、没有修改安装目录。

WK-RECOVER-TRIAL已消费当前SDK：trial(token,requestId?)与trialStateByRequest(requestId)。请求为随机UUID（1..128bytes）；首答复timeout先查询同owner原请求；查询失败保留unknown，点击查询试载仅重查，不重新trial。找到原handle后使用stopTrial并等待cleanupConfirmed。公共契约同owner+digest精确重放包含stopped，不同digest冲突；16个host-lifetime保留记录不evict，不跨host重启。

维护者报告DiY2ld前7项真实host RPC局部证明通过；outer因另一capture错误文本探针过严为1，不能声称整suite通过。本工坊新增两项协议stub回归通过，但不是工坊托管恢复验收。

未验的宿主行为均列README。这里只陈述调用方式，不将mock Host callback测试冒充真实托管包验收。
