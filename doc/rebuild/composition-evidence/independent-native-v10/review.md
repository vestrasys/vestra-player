# V10 窗态/全屏独立有限验证

2026-10-17，操作者 `rebuild_bootstrap_review`，root分配唯一CUA时段，维护者/root不并发GUI。仅绑定一次已确认存活的 `local.vestra.rebuild.shared-ui-v10`、原PID87997。未构建、未改宿主/SDK或仓库样例；只注入并恢复本次staging自有Slint故障。[启动](launch.json)、[原始stderr](stderr.log)、[stdout](stdout.log)及故障/备份同目录。CUA截图仅在会话工具输出，未冒称仓库持久截图。

## 结论

**普通显隐偏好及部分全屏状态路径有实证；全屏布局未通过，owner系统清理还有具体反例，完整窗态验收未通过。** 不用纯Policy单测或fullscreen布尔代替原生画面。没有重跑V8全部面板/菜单场景。

## 实际结果

| 场景 | 独立观察与边界 |
| --- | --- |
| 普通基线 | 底栏/侧栏同窗，点Second row显示second；40.52秒/frame1219截图有实际运动画面。公开窗口960×600逻辑像素、canvasAvailable944×474、sidebar280，preferred/effective均true |
| 普通bottom偏好 | 菜单Toggle bottom preference后底栏从原生树消失；公开preferred=false/effective=false/reason=userHidden。画布可用高变522，sidebar仍在 |
| false偏好跨FS | 进入全屏请求后底栏保持userHidden；Tab活动不擅自改成显示；sidebar preferred=true/effective=false/reason=fullscreen。退出后普通底栏仍隐藏，sidebar及second选择恢复 |
| true偏好与idle | 恢复普通bottom偏好true，再F11进入全屏；静置明确4秒后原生树仅image、公开bottom preferred=true/effective=false/reason=idle/overlay=true。Tab活动后Pause、路径、Bar focus lock重新出现在AX树。仅证明这次3秒策略路径，不测量精准隐藏延迟 |
| 全屏原生布局 | **失败。** 两轮FS截图均见大块白色外区，视频只在中间较小区域更新且左侧有残影；截图1920×1200，日志持续logical960×600。未独立获取OS窗口bounds，不能由这两个数字自行断言实际OS尺寸映射原因。fullscreen来自Slint请求事实，不证明OS已正确完成布局 |
| 返回普通窗口 | Escape关闭菜单、随后Escape退出FS，普通布局恢复；207.04秒/frame6210截图底栏/侧栏/视频正常，second选择保留。普通Pause/路径/Bar focus lock完整可见，没有明显裁字或裁控件；未据此确认声明min-height60已落实 |
| inline菜单外部关闭 | 普通窗口Commands打开菜单后点击video的AX image，菜单实际消失；随后window.state.menuOpen=false，与可见状态一致。Escape关闭FS菜单也观察到原生树移除菜单 |
| 来源连续 | 本次未执行pause/seek/open；普通与FS可见视频图案继续变化，退出后时钟/帧推进。原始source事实另提取[摘要](source-summary.json)，不将后台计数单独当屏幕或音频证据 |

## 尚未验证与工具限制

缩窗两次CUA drag均返回server `-10005 noWindowsAvailable`，其中一次先用Escape激活窗口；AX/截图仍正常且逻辑大小未变。因此没有验证space自动收起/放大恢复，未用伪造viewport或其他UI工具绕过。

FS bar恢复后点击Bar focus lock返回 `elementHasNoFrame`。在已失败的FS布局上没有继续扩大操作；栏内焦点/拖动超过3秒抑制、菜单抑制、FS临时展开sidebar和退出后偏好恢复的完整组合仍未验。声音、PCM/设备连续性、系统缩放/跨平台未验。

## 清理错误实收窄补验：未通过

在普通大窗且两个owner仍在时，仅把本次staged底栏 `bar.slint` 改成保存于 `bar.slint.review-fault` 的坏文本，然后插件菜单Disable sidebar。侧栏从原生树消失，243.78秒/frame7309仍在播放；窗口事件sidebar=null，但shellGeneration仍3。之后恢复staged原文，备份/fault字节已保留。

**原始stdio没有收到ui-owner-cleanup或host.error，不能说故障已正确反馈。** 本目录保存的staged-host-ui-main.ts确有host.error订阅；维护者同时核对binary路径对应代码有eprintln，因此此次不能简单归因于没订阅。

维护者只读定位具体候选原因，且与修前审查代码一致：`Event::Exited(owner) => self.disable(owner)`；修前disable无“owner已不在plugins则直接返回”的幂等入口，先对mount_queue执行 `retain(|c| c.reply.owner != owner)`，并清同owner preparing。第一次显式停用已remove mount并排 `Reply { owner, id: 0 }` 系统重编；Node正常EOF第二次进入时，上述retain/清preparing会删除这项系统清理，而mount早已移除，不会重新排它。于是剩余坏文件可能根本没有被重新编译，当然也没有编译失败事件。该代码路径足以解释反例，仍需修后真实回归证明；收尾只读已见维护者新增plugins存活幂等guard，但本审查尚未执行修后版本。

本次确定事实是“立即隐藏成功、系统清理/错误实收没有证据”，保留为反例。V8同形场景也不能据其隐藏成功追认为坏剩余源码的编译失败路径已完成；不倒推V8当次必然是同一调度根因。

后续只需修幂等停用/系统cleanup保留并做该单个真实故障回归，无需因而重跑全部V8/V10界面。

## 退出与GUI交还

最后读取新鲜完整AX得到Close索引10，单独执行click(10)，**未追加getAXState/getApp**。随后只读ps确认原87997及本stage所有进程均已不存在，没有V8那种工具观察导致的无参重启。没有退出码句柄，故不虚报host exit0。GUI已明确交还root/维护者，此后未继续操作或启动实例。
