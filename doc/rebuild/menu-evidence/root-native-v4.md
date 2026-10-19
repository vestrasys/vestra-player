# V4有限原生复验

操作者root，实际CUA鼠标/键盘/滑条操作。应用`local.vestra.rebuild.shared-ui-v4`，由`stage-native-evidence.mjs`暂存本轮二进制、runtime/SDK及host-ui-demo，唯一host81784/runner81790；不是八产品实现。原始stdio在`v4-native/`，截图/AX在本轮CUA会话，未另存原始CUA PNG。修前panic和启动挂起证据保留。

| 场景 | 真实观察 |
| --- | --- |
| 菜单中文 | 10.27秒/frame312截图明确绘出文件、播放、视频及禁用项原因，修前缺字现象本机已恢复；ICU告警仍存在 |
| 文件入口与取消 | 点击菜单Open确实打开macOS系统面板。明确等待11秒后点Cancel，回主窗45.95秒/frame1379；原始RPC返回cancelled=true、paths=[]，未触发旧panic或10秒超时 |
| 右键与选择 | 视频右键出现相同四项候选树，选择Open；在真实面板用cmd+shift+g及路径输入选择本轮`/tmp/vestra-bootstrap-long.mp4`，实际点击Open。RPC返回cancelled=false、路径`/private/tmp/vestra-bootstrap-long.mp4`；主窗仍session1、101.82秒/frame3052。此示例只展示路径，不隐式重新open来源 |
| 独立工具 | Processing specimen打开独立工具Window，未替换主窗口。亮度滑条0→0.4经真实AX操作、实际source.state及processing.replace；主窗截图明显变亮 |
| 暂停像素 | 主窗通过菜单暂停在140.31秒/frame4208；切工具将亮度0.4→-0.3，再切主窗截图显著变暗，仍140.31秒/frame4210，内嵌画面时间码保持20.200。说明当前同帧确实重处理，不只command Ok |
| 音频滑条 | 恢复播放后真实鼠标四次来回拖动，值依次约4.4109、-4.5743、4.9307、-3.0891；随后主窗session1、160.83秒/frame4825，真实画面变化。只证明操作和前后进展，不证明全程无短暂冻结或声音效果 |
| 工具关闭重开 | 关闭后主窗173.28秒/frame5197；再次从菜单show工具，保亮度-0.3、音频-3.089109，未新建来源 |
| 退出 | 先关工具再关主窗；CUA应用清单及ps均确认host/runner已退出 |

边界：

- 文件面板截图针对系统面板，未同时捕获驻留期间主窗draw；前后时钟进展不能代替该期间屏幕连续性证据。未验证saveFile/openFiles/openDirectory或系统失败与取消分类。
- 暂停像素操作在首次创建音频处理链**之前**，不是公共owner路径下含规范化复杂图的完整暂停回归。需要另一个最小case，不能并入先前独立滤镜实验来虚构覆盖。
- 四次drag是实际反复操作，不是单次持续按住5秒；未量最大draw间隔，未采集设备声音/逐项音频PCM，无音画同步定量结论。该示例不代替产品插件应有的连续编辑调度。
- 菜单当前仍是扁平路径候选，不是最终完整层级菜单/快捷键/焦点规范；双槽组合也还在另一实验路径。UI实验不能相加成已交付整合产品。
- 本机系统字体路径只用于实验，未复制分发字体，也不承诺跨平台字体或消除ICU分词告警。
