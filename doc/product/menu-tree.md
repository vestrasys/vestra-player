# 菜单树（生成）

2026-10-24 从 main 生成：18 个内置插件 + 官方插件 youtube、bilibili + 宿主命令，按宿主排序规则（order，内置顶层菜单固定位置）。`‹…›` 是注册该项的插件，`‹核心›`/`‹宿主›` 是播放器自身；`[…]` 是默认快捷键；`────` 是分组分隔线；只有快捷键、没有菜单项的 32 条命令不在树里。

```text
右键菜单
├─ 打开文件…  [Ctrl+O / F3]   ‹playback›
├─ 打开 ▸
│  ├─ 打开链接…  [Ctrl+U]   ‹playback›
│  ├─ 打开文件夹…  [F2]   ‹playlist›
│  ├─ YouTube 视频…   ‹youtube›
│  ├─ Bilibili 视频…   ‹bilibili›
│  ├─ 打开字幕…  [Alt+O]   ‹subtitles›
│  ├─ 载入外部音频…   ‹audio›
│  ├─ ────────
│  ├─ 最近播放 ▸
│  │  ├─ （无记录）   ‹history›
│  │  ├─ ────────
│  │  ├─ 移除无效记录   ‹history›
│  │  ├─ 清除记录   ‹history›
│  │  └─ 记录播放历史   ‹history›
│  ├─ ────────
│  └─ 添加次字幕…   ‹subtitles›
├─ 关闭  [F4]   ‹核心›
├─ 重开当前/最后文件  [Ctrl+Y]   ‹playback›
├─ 复制到剪贴板 ▸
│  ├─ 文件名   ‹playback›
│  ├─ 文件夹路径   ‹playback›
│  └─ 完整路径   ‹playback›
├─ ────────
├─ 播放 ▸
│  ├─ 播放/暂停  [Space]   ‹核心›
│  ├─ 上一个  [PageUp / Mouse:X1]   ‹核心›
│  ├─ 下一个  [PageDown / Mouse:X2]   ‹核心›
│  ├─ ────────
│  ├─ 跳转 ▸
│  │  ├─ 上一帧  [D]   ‹核心›
│  │  ├─ 下一帧  [F]   ‹核心›
│  │  ├─ 上一关键帧  [Ctrl+Shift+ArrowLeft]   ‹playback›
│  │  ├─ 下一关键帧  [Ctrl+Shift+ArrowRight]   ‹playback›
│  │  ├─ ────────
│  │  ├─ 后退5秒  [ArrowLeft]   ‹核心›
│  │  ├─ 前进5秒  [ArrowRight]   ‹核心›
│  │  ├─ 后退30秒  [Ctrl+ArrowLeft]   ‹playback›
│  │  ├─ 前进30秒  [Ctrl+ArrowRight]   ‹playback›
│  │  ├─ 后退60秒  [Shift+ArrowLeft]   ‹playback›
│  │  ├─ 前进60秒  [Shift+ArrowRight]   ‹playback›
│  │  ├─ 后退300秒  [Ctrl+Alt+ArrowLeft]   ‹playback›
│  │  ├─ 前进300秒  [Ctrl+Alt+ArrowRight]   ‹playback›
│  │  ├─ ────────
│  │  ├─ 上一句对白  [Home]   ‹subtitles›
│  │  ├─ 下一句对白  [End]   ‹subtitles›
│  │  ├─ 当前字幕起点  [Ctrl+Home]   ‹subtitles›
│  │  ├─ ────────
│  │  ├─ 跳到开头  [Backspace]   ‹playback›
│  │  ├─ ────────
│  │  └─ 时间跨度设置…   ‹playback›
│  ├─ ────────
│  ├─ 跳转到…  [G]   ‹playback›
│  ├─ 速度 ▸
│  │  ├─ (单选) 正常  [Z]   ‹核心›
│  │  ├─ 减速 0.1  [X]   ‹核心›
│  │  ├─ 加速 0.1  [C]   ‹核心›
│  │  ├─ (单选) 0.5倍   ‹playback›
│  │  ├─ (单选) 1.5倍   ‹playback›
│  │  ├─ (单选) 2倍   ‹playback›
│  │  └─ 自定义…   ‹playback›
│  ├─ 播放列表 ▸
│  │  ├─ 添加文件…   ‹playlist›
│  │  ├─ 添加链接…   ‹playlist›
│  │  ├─ 添加文件夹…   ‹playlist›
│  │  ├─ 循环 ▸
│  │  │  ├─ (单选) 不循环   ‹playlist›
│  │  │  ├─ (单选) 全部   ‹playlist›
│  │  │  └─ (单选) 单个   ‹playlist›
│  │  ├─ 随机   ‹playlist›
│  │  ├─ ────────
│  │  ├─ 打开列表文件…   ‹playlist›
│  │  ├─ 保存列表…   ‹playlist›
│  │  ├─ ────────
│  │  ├─ 打开文件时 ▸
│  │  │  ├─ (单选) 仅打开该文件   ‹playlist›
│  │  │  ├─ (单选) 加入相似文件   ‹playlist›
│  │  │  └─ (单选) 加入同文件夹全部文件   ‹playlist›
│  │  ├─ ────────
│  │  ├─ 快速推进到下一个   ‹playlist›
│  │  ├─ 记住列表   ‹playlist›
│  │  └─ 移除全部前确认   ‹playlist›
│  ├─ ────────
│  ├─ 播放时禁止屏保   ‹playback›
│  ├─ 播放时隐藏鼠标指针   ‹window›
│  ├─ 进度条悬停显示时间   ‹playback›
│  ├─ 进度条悬停显示缩略图   ‹playback›
│  ├─ ────────
│  ├─ 记忆播放位置 ▸
│  │  ├─ (单选) 不记忆   ‹history›
│  │  ├─ (单选) 自动继续   ‹history›
│  │  ├─ (单选) 每次询问   ‹history›
│  │  ├─ ────────
│  │  ├─ 同时记忆音频文件   ‹history›
│  │  └─ 清除所有记忆点   ‹history›
│  ├─ ────────
│  ├─ 章节/书签 ▸
│  │  ├─ 章节/书签列表…  [H]   ‹marks›
│  │  ├─ ────────
│  │  ├─ 添加书签  [P]   ‹marks›
│  │  ├─ 上一书签/章节  [Shift+PageUp]   ‹marks›
│  │  ├─ 下一书签/章节  [Shift+PageDown]   ‹marks›
│  │  ├─ 上一章节  [Ctrl+PageUp]   ‹marks›
│  │  ├─ 下一章节  [Ctrl+PageDown]   ‹marks›
│  │  ├─ ────────
│  │  └─ 在进度条上显示标记   ‹marks›
│  ├─ ────────
│  └─ A-B 区段循环 ▸
│     ├─ 设定起点  [[]   ‹marks›
│     ├─ 解除起点  [{]   ‹marks›
│     ├─ 起点后退 0.1 秒  [Ctrl+[]   ‹marks›
│     ├─ 起点前进 0.1 秒  [Alt+[]   ‹marks›
│     ├─ ────────
│     ├─ 设定止点  []]   ‹marks›
│     ├─ 解除止点  [}]   ‹marks›
│     ├─ 止点后退 0.1 秒  [Ctrl+]]   ‹marks›
│     ├─ 止点前进 0.1 秒  [Alt+]]   ‹marks›
│     ├─ ────────
│     ├─ 区段整体后退 0.1 秒  [Ctrl+Alt+[]   ‹marks›
│     ├─ 区段整体前进 0.1 秒  [Ctrl+Alt+]]   ‹marks›
│     ├─ ────────
│     ├─ 循环当前章节/书签区段  [Alt+\]   ‹marks›
│     ├─ 区段循环 开/关  [\]   ‹marks›
│     ├─ ────────
│     └─ 循环次数 ▸
│        ├─ (单选) 无限   ‹marks›
│        ├─ (单选) 1 次   ‹marks›
│        ├─ (单选) 2 次   ‹marks›
│        ├─ (单选) 3 次   ‹marks›
│        ├─ (单选) 5 次   ‹marks›
│        └─ (单选) 10 次   ‹marks›
├─ 字幕 ▸
│  ├─ 显示字幕  [Alt+H]   ‹subtitles›
│  ├─ ────────
│  ├─ 选择字幕轨 ▸
│  │  ├─ (单选) 不显示   ‹subtitles›
│  │  ├─ ────────
│  │  └─ 依次切换  [Alt+L]   ‹subtitles›
│  ├─ ────────
│  ├─ 次字幕 ▸
│  │  ├─ (单选) 不显示   ‹subtitles›
│  │  ├─ ────────
│  │  └─ 位置 ▸
│  │     ├─ (单选) 顶部   ‹subtitles›
│  │     └─ (单选) 主字幕上方   ‹subtitles›
│  ├─ ────────
│  ├─ 字幕样式 ▸
│  │  ├─ 粗体  [Alt+B]   ‹subtitles›
│  │  ├─ 字号 +  [Alt+PageUp]   ‹subtitles›
│  │  ├─ 字号 −  [Alt+PageDown]   ‹subtitles›
│  │  ├─ ────────
│  │  ├─ 上移  [Alt+ArrowUp]   ‹subtitles›
│  │  ├─ 下移  [Alt+ArrowDown]   ‹subtitles›
│  │  ├─ 复位位置  [Alt+Home]   ‹subtitles›
│  │  ├─ ────────
│  │  ├─ 阴影   ‹subtitles›
│  │  └─ 字幕边框不透明   ‹subtitles›
│  ├─ ────────
│  ├─ 开启 ASS/SSA 原样式   ‹subtitles›
│  ├─ ────────
│  ├─ 同步 ▸
│  │  ├─ 超前 0.5 秒  [.]   ‹subtitles›
│  │  ├─ 滞后 0.5 秒  [,]   ‹subtitles›
│  │  ├─ 复位  [/]   ‹subtitles›
│  │  ├─ ────────
│  │  └─ 手动调整同步…   ‹subtitles›
│  ├─ ────────
│  ├─ 在线字幕 ▸
│  │  ├─ 搜索与下载字幕…   ‹subtitle-online›
│  │  └─ 在线字幕设置…   ‹subtitle-online›
│  ├─ ────────
│  ├─ 重载字幕  [Ctrl+Alt+Y]   ‹subtitles›
│  ├─ 刷新字幕轨   ‹subtitles›
│  ├─ 字幕微调…   ‹subtitles›
│  └─ 字幕设置…   ‹subtitles›
├─ ────────
├─ 视频 ▸
│  ├─ 像素着色 ▸
│  │  ├─ 使用着色器  [Shift+P]   ‹video›
│  │  ├─ 着色器列表…   ‹video›
│  │  ├─ 打开着色器文件夹   ‹video›
│  │  ├─ 重新载入着色器   ‹video›
│  │  ├─ ────────
│  │  ├─ HDR 色调映射 ▸
│  │  │  ├─ (单选) 自动   ‹video›
│  │  │  ├─ (单选) 关   ‹video›
│  │  │  ├─ (单选) BT.2390   ‹video›
│  │  │  ├─ (单选) Hable   ‹video›
│  │  │  ├─ (单选) Mobius   ‹video›
│  │  │  ├─ (单选) Reinhard   ‹video›
│  │  │  ├─ (单选) 裁剪   ‹video›
│  │  │  ├─ ────────
│  │  │  └─ 动态峰值检测   ‹video›
│  │  └─ HDR 设置…   ‹video›
│  ├─ 反交错 ▸
│  │  ├─ 按序切换  [Ctrl+I]   ‹video›
│  │  ├─ (单选) 自动   ‹video›
│  │  ├─ (单选) 总是使用   ‹video›
│  │  ├─ (单选) 不使用   ‹video›
│  │  ├─ ────────
│  │  └─ 输出 ▸
│  │     ├─ (单选) 按帧   ‹video›
│  │     └─ (单选) 按场（倍帧率）   ‹video›
│  ├─ ────────
│  ├─ 比例 ▸
│  │  ├─ (单选) 保持宽高比   ‹video›
│  │  ├─ (单选) 不保持宽高比   ‹video›
│  │  ├─ (单选) 填满窗口（裁去多余）   ‹video›
│  │  ├─ ────────
│  │  ├─ 依次切换比例  [Shift+F6]   ‹video›
│  │  ├─ (单选) 默认（按来源）   ‹video›
│  │  ├─ (单选) 4:3   ‹video›
│  │  ├─ (单选) 16:9   ‹video›
│  │  ├─ (单选) 16:10   ‹video›
│  │  ├─ (单选) 1.85:1   ‹video›
│  │  ├─ (单选) 2.35:1   ‹video›
│  │  ├─ (单选) 自定义（x:y）   ‹video›
│  │  └─ 自定义比例…   ‹video›
│  ├─ 裁剪/拉伸 ▸
│  │  ├─ (单选) 不使用   ‹video›
│  │  ├─ (单选) 按 4:3 拉伸   ‹video›
│  │  ├─ (单选) 按 16:9 拉伸   ‹video›
│  │  ├─ (单选) 按 16:10 拉伸   ‹video›
│  │  ├─ (单选) 按 1.85:1 拉伸   ‹video›
│  │  ├─ (单选) 按 2.35:1 拉伸   ‹video›
│  │  ├─ (单选) 按 4:3 裁剪   ‹video›
│  │  ├─ (单选) 按 16:9 裁剪   ‹video›
│  │  ├─ (单选) 按 16:10 裁剪   ‹video›
│  │  ├─ (单选) 按 1.85:1 裁剪   ‹video›
│  │  ├─ (单选) 按 2.35:1 裁剪   ‹video›
│  │  ├─ ────────
│  │  ├─ 位置复位   ‹video›
│  │  ├─ 左移   ‹video›
│  │  ├─ 右移   ‹video›
│  │  ├─ 上移   ‹video›
│  │  └─ 下移   ‹video›
│  ├─ 帧位 ▸
│  │  ├─ 帧位复位  [Numpad5 / Alt+Backspace]   ‹video›
│  │  ├─ 全帧 +  [Numpad9 / Alt+Plus]   ‹video›
│  │  ├─ 全帧 -  [Numpad1 / Alt+-]   ‹video›
│  │  ├─ ────────
│  │  ├─ 偏移 →  [Ctrl+Numpad6 / Alt+ArrowRight]   ‹video›
│  │  ├─ 偏移 ←  [Ctrl+Numpad4 / Alt+ArrowLeft]   ‹video›
│  │  ├─ 偏移 ↑  [Ctrl+Numpad8 / Alt+ArrowUp]   ‹video›
│  │  ├─ 偏移 ↓  [Ctrl+Numpad2 / Alt+ArrowDown]   ‹video›
│  │  └─ 帧位居中  [Ctrl+Numpad5]   ‹video›
│  ├─ ────────
│  ├─ 图像处理 ▸
│  │  ├─ 上下翻转  [Ctrl+P]   ‹video›
│  │  ├─ 左右翻转  [Ctrl+Z]   ‹video›
│  │  ├─ ────────
│  │  ├─ 高斯模糊  [Ctrl+B]   ‹video›
│  │  ├─ 锐化  [Ctrl+R]   ‹video›
│  │  ├─ 边缘锐化   ‹video›
│  │  ├─ ────────
│  │  ├─ 去块  [Ctrl+H]   ‹video›
│  │  ├─ 消色带   ‹video›
│  │  ├─ 3D 降噪  [Ctrl+N]   ‹video›
│  │  ├─ 渐进降噪   ‹video›
│  │  ├─ 瞬态降噪  [Ctrl+M]   ‹video›
│  │  ├─ ────────
│  │  ├─ 倍帧   ‹video›
│  │  └─ 运动补偿   ‹video›
│  ├─ 图像旋转 ▸
│  │  ├─ 按序切换  [Alt+K]   ‹video›
│  │  ├─ (单选) 不旋转   ‹video›
│  │  ├─ (单选) 90°   ‹video›
│  │  ├─ (单选) 180°   ‹video›
│  │  └─ (单选) 270°   ‹video›
│  ├─ ────────
│  ├─ 截图 ▸
│  │  ├─ 保存当前画面  [Ctrl+E]   ‹capture›
│  │  ├─ 当前画面另存为…   ‹capture›
│  │  ├─ 复制当前画面  [Ctrl+C]   ‹capture›
│  │  ├─ ────────
│  │  ├─ 保存实际画面  [Ctrl+Alt+E]   ‹capture›
│  │  ├─ 实际画面另存为…   ‹capture›
│  │  ├─ 复制实际画面  [Ctrl+Alt+C]   ‹capture›
│  │  ├─ ────────
│  │  ├─ 连拍…  [Ctrl+G]   ‹capture›
│  │  ├─ 导出片段…   ‹capture›
│  │  ├─ 导出任务…   ‹capture›
│  │  ├─ ────────
│  │  ├─ (单选) 截取为 PNG   ‹capture›
│  │  ├─ (单选) 截取为 JPEG   ‹capture›
│  │  ├─ (单选) 截取为 BMP   ‹capture›
│  │  ├─ ────────
│  │  ├─ 实际画面包含字幕   ‹capture›
│  │  ├─ 截图设置…   ‹capture›
│  │  ├─ ────────
│  │  ├─ 截图文件夹…   ‹capture›
│  │  └─ 打开截图文件夹   ‹capture›
│  ├─ ────────
│  ├─ 画面调节…   ‹video›
│  ├─ 复位  [Q]   ‹video›
│  ├─ 亮度 -1%  [W]   ‹video›
│  ├─ 亮度 +1%  [E]   ‹video›
│  ├─ 对比度 -1%  [R]   ‹video›
│  ├─ 对比度 +1%  [T]   ‹video›
│  ├─ 饱和度 -1%  [Y]   ‹video›
│  ├─ 饱和度 +1%  [U]   ‹video›
│  ├─ 色调 -1%  [I]   ‹video›
│  └─ 色调 +1%  [O]   ‹video›
├─ ────────
├─ 声音 ▸
│  ├─ 音量 +  [ArrowUp / Mouse:WheelUp]   ‹核心›
│  ├─ 音量 −  [ArrowDown / Mouse:WheelDown]   ‹核心›
│  ├─ 静音  [M]   ‹核心›
│  ├─ ────────
│  ├─ 音频工作台…   ‹audio›
│  ├─ 音轨 ▸
│  │  ├─ 没有音轨   ‹audio›
│  │  ├─ ────────
│  │  ├─ 按序切换  [Alt+A]   ‹audio›
│  │  ├─ ────────
│  │  ├─ (单选) 立体声   ‹audio›
│  │  ├─ (单选) 左声道   ‹audio›
│  │  ├─ (单选) 右声道   ‹audio›
│  │  ├─ (单选) 单声道混合   ‹audio›
│  │  └─ (单选) 左右互换   ‹audio›
│  ├─ 输出设备 ▸
│  │  └─ 没有可用的输出设备   ‹audio›
│  ├─ ────────
│  ├─ 均衡器  [Shift+E]   ‹audio›
│  ├─ 预设 ▸
│  │  ├─ (单选) 平直   ‹audio›
│  │  ├─ (单选) 人声   ‹audio›
│  │  ├─ (单选) 音乐   ‹audio›
│  │  └─ (单选) 影院   ‹audio›
│  ├─ 声音同步 ▸
│  │  ├─ 超前 50 毫秒  [>]   ‹audio›
│  │  ├─ 滞后 50 毫秒  [<]   ‹audio›
│  │  ├─ 复位  [?]   ‹audio›
│  │  ├─ ────────
│  │  ├─ (单选) 不保持   ‹audio›
│  │  ├─ (单选) 本次运行内保持   ‹audio›
│  │  └─ (单选) 始终保持   ‹audio›
│  ├─ 声音处理 ▸
│  │  ├─ 规格化  [Shift+N]   ‹audio›
│  │  └─ 晶化  [Shift+C]   ‹audio›
│  ├─ 复位   ‹audio›
│  ├─ ────────
│  └─ 纯音频时显示专辑封面   ‹visualizer›
├─ 模拟器 ▸
│  ├─ 打开 NES ROM…   ‹nes›
│  ├─ 停止 NES   ‹nes›
│  └─ NES 工具…   ‹nes›
├─ ────────
├─ 全屏  [Enter / F11]   ‹宿主›
├─ 屏幕 ▸
│  ├─ 画中画   ‹window›
│  ├─ 迷你尺寸  [`]   ‹window›
│  ├─ ────────
│  ├─ (单选) 0.5倍  [1]   ‹window›
│  ├─ (单选) 1倍  [2]   ‹window›
│  ├─ (单选) 1.5倍  [3]   ‹window›
│  ├─ (单选) 2倍  [4]   ‹window›
│  ├─ 适应视频比例  [8]   ‹window›
│  ├─ ────────
│  ├─ 开始播放时 ▸
│  │  ├─ (单选) 不调整   ‹window›
│  │  ├─ (单选) 0.5倍   ‹window›
│  │  ├─ (单选) 1倍   ‹window›
│  │  ├─ (单选) 1.5倍   ‹window›
│  │  ├─ (单选) 2倍   ‹window›
│  │  ├─ (单选) 适应视频比例   ‹window›
│  │  ├─ ────────
│  │  └─ 只调整一次   ‹window›
│  ├─ ────────
│  └─ 置顶 ▸
│     ├─ 切换置顶方式  [Ctrl+T]   ‹window›
│     ├─ ────────
│     ├─ (单选) 不置顶   ‹window›
│     ├─ (单选) 总是置顶   ‹window›
│     ├─ (单选) 播放时置顶   ‹window›
│     └─ (单选) 全屏播放时置顶   ‹window›
├─ ────────
├─ 播放列表  [F6]   ‹playlist›
├─ 播放信息  [Tab]   ‹info›
├─ 播放属性…  [Ctrl+F1]   ‹info›
├─ 插件工坊…   ‹workshop›
├─ 插件商城…   ‹store›
├─ ────────
├─ 设置 ▸
│  ├─ 选项…  [F5]   ‹preferences›
│  ├─ ────────
│  ├─ 快捷键…   ‹hotkeys›
│  ├─ 鼠标…   ‹hotkeys›
│  ├─ 忽略默认快捷键   ‹hotkeys›
│  ├─ ────────
│  ├─ 插件管理…   ‹plugins›
│  ├─ ────────
│  └─ 外观 ▸
│     ├─ (单选) Vestra   ‹preferences›
│     ├─ ────────
│     ├─ 空闲时显示徽标   ‹visualizer›
│     ├─ 选择徽标图片…   ‹visualizer›
│     └─ 恢复默认徽标   ‹visualizer›
├─ ────────
├─ 关于 Vestra Player…  [F1]   ‹info›
├─ ────────
└─ 退出  [Cmd+Q]   ‹宿主›
```
