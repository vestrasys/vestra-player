# 交互验收补充：业务路径功能审计

2026-10-17。本文件记录当前实验代码的补充功能测试，不表示最终 API 定稿，也不替代[九插件原生交互验收](interactive-acceptance.md)。本审计角色不是最终独立验收者；这里只使用真实 Rust 宿主、Node 插件进程和 FFmpeg，文件选择用 headless 路径 fixture，因此证据等级为 F/B，不能证明实际鼠标命中、操作系统文件面板或物理 NES 输入。

## 执行与范围

新增测试：`src/tests/functional/business-paths.test.ts`。它补充前轮没有执行的业务边界，不把旧测试数量当作全部功能证据。

```sh
PATH=/opt/homebrew/opt/ffmpeg-full/bin:$PATH \
VESTRA_HOST="$PWD/.cache/probe-target/debug/vestra-slint-experimental1" \
node --test src/tests/functional/business-paths.test.ts
```

从 `src/` 运行 `node --test tests/functional/business-paths.test.ts` 也可，路径以测试文件定位。完整统一检查由维护者纳入 `npm test`。测试生成的媒体、字幕、授权自制 ROM、存档、PNG 与 JSON 结果均位于 `.cache/evidence/functional-audit/`，保留以供独立验收复核。

本次使用宿主 binary SHA-256：`0651467f13ce045a0e8a02473a267192903fd4ef621f4b0e1c362009d0b7cf2b`。维护者随后修改宿主时须重新回归；本记录不能自动外推为后续构建通过。

| 场景 | 实际断言与产物 | 结果 |
|---|---|---|
| 九插件同时加载、旧三插件与新六探针共存 | 9个真实worker存在；7个真实视频分3页，重复去重、跨页播放与▶标记；暂停seek保持暂停；高斯草稿9不提前改变后端2，应用才更新9；旧高斯、新画面、音频预设和字幕同时启用，经导出插件生成原始/处理PNG，独立解码平均像素差64.99；卸载高斯不清除画面插件状态；坏MP4保留原媒体，后续合法打开恢复 | 修复 FUNC-001 后通过；`legacy-nine.json` |
| 字幕导航与失败保留 | 三cue实际seek为3→5→1秒且保持暂停；正负delay、字号64和颜色状态；损坏SRT报错且保留原3条字幕，随后导航按-0.5秒delay定位2.5秒 | 通过；`subtitle-navigation.json`。像素/字幕合成由现有真实处理测试与原生验收另证 |
| 导出参数与尾部连续截图 | 数量小数、过小间隔、过长片段、非法字幕开关拒绝且保留旧值；错扩展名报错、取消输出不执行；8秒媒体7.5秒处请求3张/0.5秒间隔，只保留第1张并明确超时长错误；PNG用独立FFmpeg解码 | 通过；`capture-boundaries.json`及`tail-*-001.png` |
| NES失败不破坏会话 | 自制ROM真实打开并暂停；坏ROM不替换会话；磁盘存档有效；同路径保存明确EEXIST且逐字节保留；异ROM、坏JSON、无效结构拒绝，session与暂停状态不变；合法恢复、取消恢复、关闭资源 | 通过；`nes-negative.json`、真实磁盘存档 |
| 工坊需求文件 | 坏JSON、未知字段、8193字符输入拒绝，原需求与非busy保持；合法增量4恢复；取消选择保留需求；无历史回退错误可恢复 | 通过；`workshop-request-errors.json`。生成/Pi/事务已由现有真实工坊测试覆盖，本项不重复声称模型完整性 |

## FUNC-001：错误被普通播放状态覆盖

分类：**文件播放插件 bug**，不需要新公共 API，也不是 Slint 技术选型失败。

最小复现：加载 `file-playback`，打开合法视频并暂停，再选择一个包含纯文本的 `.mp4`。后端正确拒绝，runtime 将错误写到插件 status，但 `onPlayback` 随每次状态广播立即将 status 改为旧媒体路径。stderr 中有诊断，用户界面看不到稳定错误。补充测试首次在等待错误可见时失败，日志 `.cache/functional-audit-run.log`；未降低断言。

root 授权窄修 `src/plugins/file-playback/index.ts`：插件保存自己的 `operationError`，普通 playback tick 保留该错误；成功打开、暂停/继续或 seek 后清除，其他插件真正切换媒体也清除。取消选择保留状态，系统选择器自身错误使用同样展示路径。公共 SDK/协议/宿主不变，其他插件未修改。

修复后测试增加明确断言：错误显示后等待300ms、经过播放状态广播仍保持；坏媒体不丢原会话；随后打开合法文件 status 恢复新路径。真实5项测试通过（最新加入组合PNG断言后5/5，10.4秒，日志`.cache/functional-audit-final.log`）；路径独立性已从 `src/` 验证。旧插件worker最新回归8/8通过，包含维护者增加的31.2秒对话框等待测试，日志`.cache/functional-audit-legacy-final.log`；strict与相关文件格式检查通过，最终统一检查另行固定证据。

## 范围与仍需确认

播放列表当前仅实现加入、去重、分页、选项播放与显隐，**没有自动播放下一项/循环播放的实现或承诺**。尾部行为不能因为名为“播放列表”就推断为自动连播。若需要该产品能力，应另列需求，而不是修改当前测试假装已实现。

本轮没有通过 headless 动作声称右键菜单、每个可见控件、系统对话框、实际保存覆盖确认、物理输入或窗口关闭已通过。API 和插件 bug 必须分别记录；真实交互矩阵尚有未覆盖项时，goal 不应因本文件5项通过而结束。
