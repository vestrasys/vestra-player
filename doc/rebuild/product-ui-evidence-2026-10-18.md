# 插件组合实际界面验证（2026-10-18）

范围：基础播放与播放列表，两插件真实运行；并非所有产品最终验收。

运行物：`/tmp/vestra-product-slices-QKa2uO/Vestra Product Slices.app`，唯一 bundle ID `local.vestra.product-slices-vestra-product-slices-QKa2uO`，实际宿主 PID 32994。维护者启动并将 GUI 独占交给 root；root 用 CUA inventory 确认已运行后绑定该实例，未启动第二份程序。协议结果见该目录 `result.json`；运行命令和日志见 `runtime-command.json`、`runtime.stderr.log` 以及 `/tmp/vestra-product-combined-runtime.log`。源版本以本轮冻结副本和运行记录为准。

## 实际 UI 操作与观察

1. 点击播放栏「打开」，出现原生 Open 面板，通过 Cmd+Shift+G 选择 `/tmp/vestra-product-ui-media-h2ctenru/A-continuous.mp4`，点击 Open。出现真实彩色视频画面；AX 显示 session 1、0.77s、embedded frame 21，随后 3.80s、frame 113，播放栏显示播放中、总长30秒，控制项加载后可用。
2. 点击暂停：AX 显示 8.90s、frame268、已暂停，按钮变为播放；再点击恢复。尚未以两次相隔观察证明暂停期间帧恒定，故这里仅确认控制状态转变。
3. 点击播放列表「添加文件…」，出现题为「添加到播放列表」的原生多选面板；通过路径选择同一文件并点击 Open。列表新增1项、重复0、待重试0。未声称已验证批量多选。原 session1 在操作过程中自然结束，添加后仍显示已结束，未自动重开来源。
4. 点击列表「播放」：条目标记播放中，宿主切换到 session2、0.43s、frame914。
5. 点击「清空」：列表为空，显示「列表已清空，当前媒体继续」；来源仍是同一A文件、session2，5.30s、frame1060，播放中。验证清空队列未关闭来源。
6. 播放中点击 Commands：session2、11.30s、frame1239。展开播放子菜单后仍是session2、15.27s、frame1358，播放栏时间同步推进。菜单停留期间观察到了播放时间和渲染帧增长。
7. 点击「前进5秒」，菜单关闭，同session2继续，观察23.33s、frame1456。此次未精确测量点击瞬间的前后时间，不能据此声称跳转误差已符合某精度指标。

## 限制与异常

- 本轮参数 `--silent` 对应 ao=null，因此不证明实际扬声器输出或音频连续性。
- 原生文件面板的一些 AX 行/文本点击未选择文件，路径方式有效。曾出现路径文本意外变为 `/tmp/vestra-bootstrap-long.mp4`，root重新输入A路径并核对实际选中后才确认Open。维护者确认无其它GUI输入。尚无证据将此归为插件bug，亦不以此宣称所有原生交互成功。
- 缺少两次拖动跳转、调速、音量/静音互斥、快捷键、右键鼠标入口、队列重排/删除/自动下一项、focus恢复、错误取消等完整验证。
- Slint普通界面已真实可见。维护者日志有中文/日文分词模型警告，需要另行判断实际影响；本轮截图中文可见。
- 宿主设置有限运行时长210秒。到期退出不能被归为用户操作导致闪退。

结论：两插件的上述实际组合路径有原生界面证据；保留候选状态，不据此作完整产品验收结论。
