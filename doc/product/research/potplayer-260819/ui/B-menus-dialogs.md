# B：PotPlayer 260819 菜单与对话框（静态分析）

分析对象：`$P/x/PotPlayer64.dll`（PE32+，ImageBase `0x180000000`，`.rsrc` 文件偏移 28466176 / RVA 29130752），`$P/x/Menus/SimpleMenu.xml`，`$P/x/Language/English.ini` 与 `Chinese(Simplified).ini`，默认皮肤 `$P/skin-default/`，以及仓库已有盘点 `doc/product/research/potplayer-260819/`（下称 `inv/`）。全程只读、未执行任何 PotPlayer 程序。

证据写法：`menus.json` 节点用模板 key（如 `101_0_24`），`L####` 默认指 `English.ini` 行号；RT_DIALOG/RT_ACCELERATOR 写资源 ID 和文件偏移；代码证据写 rizin 虚拟地址。中间脚本与解码结果在 `$P/workB/`（`rsrc.py`、`dlg.py`、`symtab.py`、`msgmap.py`、`dialogs_decoded.json`、`symtab.json`、`msgmap.json`），可重跑。

## 1. 摘要（最重要的 10 条）

1. **完整右键菜单是 RT_MENU 101**，顶层 31 项（含 4 条分隔线），分 5 组：打开类（打开文件/打开/工具/收藏/关闭）→ 功能子菜单（特效/电视/DVD/播放/字幕/视频/声音/滤镜/皮肤/配置）→ 画面尺寸（帧位/比例/屏幕/全屏/全屏+）→ 窗口入口（选项 F5、列表 F6、控制 F7、属性 Ctrl+F1、关于 F1）→ 登录/退出。**确认**（`inv/menus.json` key `101_0_0`…`101_0_30`）。
2. **`SimpleMenu.xml` 是另一套精简菜单**：把播放列表、控制面板等开关收进“视图”子菜单（`$MENU_VIEW`），首项就是 `ID_PLAYLIST_WINDOW`、`ID_CONTROL_BOX_WINDOW`。默认用哪一套菜单静态无法确定（**未知**）。
3. **符号名到 CmdID 的映射表可以从代码中完整恢复**：DLL 初始化时以 `mov r8d, <id>; lea rdx, L"<名字>"; call 0x1805010d0` 注册了 1,264 个名字（如 `0x1804fc049`：`ID_PLAYLIST_WINDOW = 0x271B = 10011`；`0x1804fe740`：`ID_CONTROL_BOX_WINDOW = 0x288F = 10383`）。**确认**。
4. **播放列表开关 = 10011 `ID_PLAYLIST_WINDOW`，F6；控制面板开关 = 10383 `ID_CONTROL_BOX_WINDOW`，F7**。两者都是带勾选的切换项（MFC 更新处理函数调用 `CCmdUI::SetCheck`），快捷键由 RT_ACCELERATOR 157 绑定。**确认**（加速键表）/**推断**（勾选状态由反汇编推得）。
5. **`ID_CONTROL_BOX_WINDOW` 不是底部控制栏**，它是独立的“控制窗口”（皮肤 `ControlBoxSkinArea`，301×227 px，四个页签：音频/视频/字幕/播放），内容是均衡器+音量、亮度/对比度/饱和度/色调滑块、字幕字体/位置/同步、跳转/速度/AB 循环。PotPlayer 静态菜单里**没有“显示/隐藏底部播放栏”的项**，只有 `皮肤 → 视频下自动隐藏`（10369 `ID_SKIN_AUTO_HIDE`）。**确认**（`SubCommon.txt:189-369`，`101_0_14_7`）。
6. **菜单模板本身不带勾选标志**（旧式 MENU 模板，flags 只有 0/0x10/0x80/0x2），勾选和单选圆点都在运行时加：单选组统一走辅助函数 `0x18040f160`（先 `SetCheck`，再 `GetMenuItemInfoW`/`SetMenuItemInfoW` 置 `MFT_RADIOCHECK 0x200`）。例：截图格式 10354–10356、循环方式 10495–10497、变速方式 10396–10398、置顶方式 10038–10039 是单选；静音、均衡器、显示字幕、软模糊、全屏、列表、控制面板是复选。**推断**（反汇编）。
7. **打开工具对话框的菜单项在窗口打开时也显示勾**：手动定位 10325、连续截图 10222、录制视频 10370、字幕浏览器 10228、播放信息 10195 的更新函数都是“窗口指针非空 → `SetCheck(1)`”。**推断**（`0x1803c9050` 等）。
8. **对话框模板字体几乎全部是 Arial 9pt**（186 个模板中 182 个；另 3 个 MS Shell Dlg 8pt，1 个 돋움 9pt）。按 96 DPI 计算对话框基本单位为 7×14 px，即 **1 DLU = 1.75 px（横）、1.75 px（纵）**。常用尺寸：编辑框高 12 DLU = 21 px，复选框高 10 DLU ≈ 18 px，标签高 8 DLU = 14 px，按钮高 12 DLU = 21 px（主操作按钮 14 DLU ≈ 25 px）。**确认**（模板）/**推断**（换算依赖 Arial 度量）。
9. **弹出式对话框模板都没有 `WS_CAPTION`，也几乎没有 IDOK/IDCANCEL**（186 个中只有 306、312 两个含 id 1/2）；内容一律从 y≈20 DLU（≈35 px）开始。推断 PotPlayer 自绘标题栏和边框，确定/取消/关闭按钮在运行时补上；标题文字也不在模板里。**推断**，尺寸需实机确认。
10. **参数选项页是子窗口模板**（`WS_CHILD|DS_CONTROL`，约 284×240 DLU ≈ 497×420 px），嵌在“选项”框架 Dialog(324)（429×301 DLU ≈ 751×527 px，左侧树 117 DLU 宽）中。与我们插件相关的页：图像属性 384、图像处理 386、均衡器 363、声音处理 387、字幕字体 383、时间跨度 364。**确认**（模板）/**推断**（页与框架的对应）。

## 2. 换算基准与通用规则

### 2.1 DLU → 像素

- 模板字体：Arial，9pt，weight 400（`DLGTEMPLATEEX` 字段，见 `$P/workB/dialogs_decoded.json`）。
- 96 DPI 下 9pt → `lfHeight = -12`（12 px/em）。
- 使用 Arial 的 `OS/2.usWinAscent=1854`、`usWinDescent=434`、`unitsPerEm=2048` 计算：ascent = round(10.86) = 11，descent = round(2.54) = 3，**tmHeight = 14**。
- 平均字宽按 Windows `GdiGetCharDimensions` 的做法：取 “A–Z a–z” 52 个字符在 12 ppem 下的 `hdmx` 设备宽度之和 = 361，`(361/26 + 1)/2` 取整 = **7**。
- 公式：`px_x = MulDiv(dlu_x, 7, 4)`，`px_y = MulDiv(dlu_y, 14, 8)`，即两个方向都是 ×1.75，四舍五入。
- 字体度量取自 macOS 自带 `/System/Library/Fonts/Supplemental/Arial.ttf`（与 Windows 版 Arial 的 hdmx 应一致）——**推断**。
- 对照：MS Shell Dlg 8pt（Microsoft Sans Serif 11 ppem）计算得 6×12（Windows 上通常记为 6×13）；语言文件给出的 `DefaultMessageFont` 是 `宋体`（`Chinese(Simplified).ini:4`）和 `Segoe UI`（`English.ini:3`）。**如果 PotPlayer 在运行时把对话框字体换成语言字体，像素值会变**：宋体 9pt 约 6×12（1 DLU ≈ 1.5 px 横 / 1.5 px 纵），Segoe UI 9pt 约 7×15（1.75 / 1.875）。是否替换静态无法确定（**未知 U1**）。

### 2.2 对话框框架（所有弹出式对话框）

```css
/* 弹出式对话框（Dialog 407/310/463/461/412/337/413/424/403/324 等）*/
.pot-dialog {
  font: 9pt "Arial";            /* 模板字体；运行时可能被替换，见 U1 */
  /* style 位：WS_POPUP | DS_SETFONT（多数），部分加 WS_SYSMENU / DS_CENTER / WS_CLIPCHILDREN */
  /* 模板无 WS_CAPTION、无标题文字 → 自绘标题栏（推断） */
  padding-top: 35px;            /* 首个控件 y = 20–25 DLU（35–44 px），留给自绘标题栏（推断） */
  padding-inline: 14px 14px;    /* 左边距 8–15 DLU（14–26 px），右侧与之对称 */
}
.pot-dialog .content-start { top: 20dlu; }  /* 337/413: y=20；310/324: y=21；407: y=23；403: y=24；463: y=25 */
```

证据：style 字段（如 407 `0x80080040`，310 `0x86000840`，324 `0x80000040`）；含 IDOK/IDCANCEL 的仅 306、312（`dlg.py` 统计）；`AFX_DIALOG_LAYOUT` 86 个均为空（无 MFC 动态布局）。

### 2.3 通用控件尺寸（全部 186 个模板的统计，`dlg.py`）

| 控件 | 最常见尺寸（DLU） | 像素（96 DPI） | 计数 |
| --- | --- | --- | --- |
| 编辑框高度 | 12 | 21 | 344/364 |
| 复选/单选框高度 | 10（其次 9） | 17.5（16） | 458/510 |
| 静态标签高度 | 8 | 14 | 714/1011 |
| 普通按钮 | 38×12、59×12、40×12、43×12、35×12 | 66×21、103×21、70×21、75×21、61×21 | 35/34/20/13/10 |
| “…”浏览按钮 | 15×11、17×12 | 26×19、30×21 | 34 |
| 主操作按钮（底部） | 高 13–14 | 23–25 | 例：324、337、413、424 |
| 下拉框（含展开高度） | 高 12 显示，模板高 52–179 为下拉列表高度 | | 320 |
| UpDown 旋钮 | 10–11×12–14，叠在编辑框右缘 | 18–19×21–24 | 206 |
| Trackbar 横向 | 高 10 | 18 | 多见 |

```css
.pot-dialog button          { height: 21px; min-width: 66px; }   /* 12 DLU 高，38 DLU 宽 */
.pot-dialog button.primary  { height: 24px; }                    /* 14 DLU，用于“应用/开始/复制到剪贴板”等底部按钮 */
.pot-dialog button.browse   { width: 26px; height: 19px; }       /* "..." 15×11 DLU */
.pot-dialog input[type=text]{ height: 21px; }                    /* 12 DLU */
.pot-dialog .check-row      { height: 18px; margin-bottom: 1–2px; } /* 10 DLU 高，行距 10–11 DLU = 17.5–19 px */
.pot-dialog label           { height: 14px; text-align: right; } /* 8 DLU；表单左列标签多为 SS_RIGHT */
.pot-dialog .row-pitch      { gap: 26px; }                       /* 表单行距 15–16 DLU = 26–28 px（384/364/387） */
.pot-dialog fieldset        { padding: 21px 10px 10px; }         /* 分组框标题到首行约 11–12 DLU，左内缩 5–8 DLU */
.pot-dialog hr.etched       { height: 2px; }                     /* SS_ETCHEDHORZ，1–2 DLU */
```

## 3. 右键菜单

### 3.1 菜单资源总览（`inv/menus.json`，flags 为模板原值）

| RT_MENU | 根 | 节点数 | 用途（推断） |
| --- | --- | --- | --- |
| 100 | `Menu`（20010） | 1 | 占位 |
| 101 | `Popup Menu` | 895 | **主右键菜单（完整版）** |
| 102 | `Popup Menu` | 9 | 编辑框右键（撤销/剪切/复制/粘贴/删除/全选） |
| 105 | `Size` | 8 | 固定窗口尺寸列表 |
| 106 | `Equlizer Preset` | 6 | 均衡器预设操作：添加…/保存/移除/—/初始化（10181–10184） |
| 107 | `Menu` | 14 | 屏幕采集区菜单 |
| 111 | `PL` | 73 | **播放列表面板右键菜单** |
| 115 | `Popup Menu` | 12 | 精简菜单（播放/停止/音量/静音/比例/全屏/属性/关于）——用途未定，可能是托盘或音频模式 |
| 449 | `Popup Menu` | 3 | 共享/举报 |
| 450 | `Popup Menu` | 7 | 文件导航器窗格 |

模板 flags：`0x10` = MF_POPUP，`0x80` = MF_END（每级最后一项），`0x2` = MF_GRAYED（仅 107 的两个标题项和 450 的两个窗格名）。**模板中没有任何 MF_CHECKED（0x8）**，勾选全部是运行时状态。**确认**。

### 3.2 主右键菜单（101）顶层顺序与分组

| # | key | 中文 / English | CmdID / 子菜单 | 快捷键（模板文字） | 行号 |
| --- | --- | --- | --- | --- | --- |
| 0 | 101_0_0 | 打开文件... / Open File(s)... | 10443 `ID_BASE_OPEN` | F3 | L2491 |
| 1 | 101_0_1 | 打开 / Open | ▶ | | L2492 |
| 2 | 101_0_2 | 工具栏 / Tools | ▶ | | L2528 |
| 3 | 101_0_3 | 收藏 / Album/Favorites | ▶ | | L2551 |
| 4 | 101_0_4 | 关闭 / Close Playback | 10167 `ID_CLOSE_MEDIA` | F4 | L2555 |
| — | 101_0_5 | 分隔线 | | | |
| 6 | 101_0_6 | 特效 / Visualization | ▶ | | L2556 |
| 7 | 101_0_7 | 电视 / TV (WDM/BDA) | ▶ | | L2567 |
| 8 | 101_0_8 | DVD / DVD Control | ▶ | | L2584 |
| 9 | 101_0_9 | 播放 / Playback | ▶ | | L2611 |
| 10 | 101_0_10 | 字幕 / Subtitles | ▶ | | L2747 |
| 11 | 101_0_11 | 视频 / Video | ▶ | | L2822 |
| 12 | 101_0_12 | 声音 / Audio | ▶ | | L3026 |
| 13 | 101_0_13 | 滤镜 / Filters | ▶ | | L3089 |
| 14 | 101_0_14 | 皮肤 / Skins | ▶ | | L3091 |
| 15 | 101_0_15 | 配置/语言/其他 / Misc | ▶ | | L3113 |
| — | 101_0_16 | 分隔线 | | | |
| 17 | 101_0_17 | 帧位 / Frame Size | ▶ | | L3154 |
| 18 | 101_0_18 | 比例 / Aspect Ratio | ▶ | | L3179 |
| 19 | 101_0_19 | 屏幕 / Window Size | ▶ | | L3211 |
| 20 | 101_0_20 | 全屏 / Fullscreen (Keep AR) | 10013 `ID_FULLSCREEN` | Enter | L3241 |
| 21 | 101_0_21 | 全屏+ / Fullscreen (Stretch) | 10308 `ID_FULL_FIT_SCREEN` | Ctrl+Enter | L3242 |
| — | 101_0_22 | 分隔线 | | | |
| 23 | 101_0_23 | 选项... / Preferences... | 10010 `ID_OPTION_WINDOW` | F5 | L3243 |
| 24 | 101_0_24 | 列表... / Playlist... | 10011 `ID_PLAYLIST_WINDOW` | F6 | L3244 |
| 25 | 101_0_25 | 控制... / Control Panel... | 10383 `ID_CONTROL_BOX_WINDOW` | F7 | L3245 |
| 26 | 101_0_26 | 属性... / Playback/System Info... | 10195 `ID_PLAYINFO_WINDOW` | Ctrl+F1 | L3246 |
| 27 | 101_0_27 | 关于... / About... | 57664 `ID_APP_ABOUT` | F1 | L3247 |
| — | 101_0_28 | 分隔线 | | | |
| 29 | 101_0_29 | 登录... / Login... | 10187 `ID_LOGIN_MENU` | | L3248 |
| 30 | 101_0_30 | 退出 / Exit | 57665 `ID_APP_EXIT` | Alt+F4 | L3249 |

要点：
- 一级菜单里同时放直接命令（打开文件、关闭、全屏、窗口入口）和领域子菜单，没有单独的“文件”子菜单；“打开”子菜单承担文件/链接/文件夹/设备/字幕/外部音频的打开（`101_0_1_*`）。
- 菜单文字里 `\t` 后的快捷键只是显示文本；实际绑定见 RT_ACCELERATOR（§3.6）。二者大多一致，例外：`播放 → 上一文件/下一文件` 标 PgUp/PgDn（10067/10068），而加速键表中 PgUp/PgDn 绑定 10123 `ID_PROCESS_PREV` / 10124 `ID_PROCESS_NEXT`。**确认**（两处数据不同）。

### 3.3 与我们相关的子菜单结构（节选，分隔线用 —— 表示）

```
播放 (101_0_9)
  播放|暂停 10014 Space
  ——
  上一文件 10067 PgUp / 下一文件 10068 PgDn
  ——
  章节/书签 10324 H
  定位 ▶  上一帧 D / 下一帧 F / 上一关键帧 / 下一关键帧
          —— 步退%s 10059 ← / 步进%s 10060 → / Ctrl+←→ 10061/10062 / Shift+←→ 10063/10064 / Ctrl+Alt+←→ 10065/10066
          —— 重新开始 / 中段 / 结束前30秒 —— 上一对白 / 下一对白 / 当前字幕起点
          —— 以关键帧定位 10265 / 手动定位... 10325 G / 时间场景浏览... / 章节场景浏览... / 字幕场景浏览...
          —— 时间跨度设置... 10758
  播放速度 ▶  正常/之前的速度 10246 Z / 减速 - 10247 X / 加速 + 10248 C
             —— 默认速度控制 10396 / 高品质速度控制(推荐) 10397 / 调速时不校正 10398   ← 单选组
  跳略播放 ▶ …
  播放列表 ▶ 专辑 ▶ / 列表管理 ▶ / 队列 ▶ / 添加/编辑 ▶（添加文件 10196、添加文件夹 10197、添加链接 10198 …）
            / 删除 ▶（移除所选 10200、移除未选 10212、移除所有 10201、移除无效 10210、删除文件…）
            / 排序 ▶ / 选择 ▶ / 样式 ▶ —— 播放列表设置... 10759
  AB 区段循环 ▶ … / 循环播放 ▶（切换 10034 / 不循环 10495 / 全部循环 10496 / 单个循环 10497 ← 单选）
  无缝播放 ▶ …
  ——
  无序(随机)播放 10069 / 记忆播放位置 10419 / 播放时隐蔽指针 10121 / 播放时禁止屏保 10122 / …   ← 复选
  ——
  鼠标在进度条上时显示时间 10812 / 显示缩略图 10813 / 显示书签标记 10814
  ——
  播放设置... 10757

字幕 (101_0_10)
  选择字幕 ▶（打开字幕... 10146 Alt+O / 添加字幕... 10147 / 添加次字幕... 10940 —— 依次选择字幕 29199 Alt+L）
  显示字幕 10126 Alt+H / 字幕浏览器... 10228 Alt+E / 手动输入字幕... 10266 Alt+P
  —— 输出方式两项 10220/10221 —— 字幕字体处理 ▶ / 字幕样式 ▶ / 字幕同步(帧率) ▶（手动调整同步... 10865，超前 10140 ">" / 滞后 10141 "<" / 复位 10142 "/" …）
  / 保存字幕 ▶ / 3D字幕 ▶ / 在线字幕搜索 ▶
  —— 复制当前字幕 … 字幕显隐（7 个复选）—— 字幕设置... 10760

视频 (101_0_11)
  选择图像 ▶ V / 视频渲染器 ▶ / 像素着色 ▶ S / 3D 视频输出 ▶ J / 图像处理滤镜 ▶
  —— 分辨率 ▶ / 反交错 ▶ / 裁剪/拉伸 ▶ / 下边距 ▶ / 图像处理 ▶（翻转、软模糊 10257 Ctrl+B、锐化 10256 Ctrl+R … 图像处理设置... 10529）
  / 图像旋转 ▶ / 图像截取 ▶ K（见下）/ 视频录制 ▶ / AviSynth ▶
  —— 图像属性复位 10085 Q / 亮度 -1% W / +1% E / 对比度 R/T / 饱和度 Y/U / 色彩度 I/O
  —— 视频输出设置... 10363

图像截取 (101_0_11_12)
  创建缩略图... 10353 Alt+N —— 连续截图... 10222 Ctrl+G / 立即连续截图 10431 / 选择截图存档路径... 10334 / 打开截图存档文件夹 10225 / 条件允许时储存到播放文件夹 10913
  —— 复制当前源画面 10223 Ctrl+C / 截存当前源画面 10224 Ctrl+E / 截取当前源画面为... 10357
  —— 复制/截存/另存当前实画面 10432/10433/10434
  —— 截取为 BMP 10354 / JPEG 10355 / PNG 10356   ← 单选
  —— 以当前时间命名 10556 / 以播放时间命名 10557 —— 同时截取字幕 10305 / 按调整后比例保存 10587 —— 删除最近的采集文件 11046

声音 (101_0_12)
  选择声音 ▶ A / 默认声音输出设备 ▶ / Pass Through ▶ / 次声音输出设备 ▶ / 20005 / 20006 / 20007（运行时占位）
  —— 播放音量控制 ▶ / 系统音量 ▶ / 音量 + 10035 Up / 音量 - 10036 Down / 静音 10037 M
  —— 均衡器 10095 Shift+E / 预置 20008（运行时占位，推断替换为预设列表）
  —— 声音匹配(同步) ▶（手动调整同步... 10878 …）/ 声音处理 ▶（规格化 10009 Shift+N、晶化、降噪、混响、语音消除、语音增强、左右声道互换）/ 音轨 ▶
  —— 开启声音处理滤镜 10454 Shift+X / 声音输出设置... 10391
```

证据：`$P/workB/tree.txt`（由 `inv/menus.json` 生成，含 key 与行号），关键行 L2611–L2746、L2747–L2821、L2822–L3025、L3026–L3088。

### 3.4 `SimpleMenu.xml`（精简菜单，`$P/x/Menus/SimpleMenu.xml`）

```
ID_BASE_OPEN(10443)                    :3
CMD_POPUPMENU_OPEN(21000)  ▶           :4
CMD_POPUPMENU_FAVORITE     ▶           :5
——                                     :6
ID_PLAY_PAUSE(10014)                   :7
ID_CLOSE_MEDIA(10167)                  :8
——                                     :9
ID_FULLSCREEN(10013)                   :10
ID_FULL_FIT_SCREEN(10308)              :11
——                                     :12
$MENU_VIEW ▶                           :13–30
   ID_PLAYLIST_WINDOW(10011)           :14
   ID_CONTROL_BOX_WINDOW(10383)        :15
   ID_BROADCAST_LIST_WINDOW(10185)     :16
   ID_CHAT_WINDOW(10012)               :17
   ID_BROADCAST_TOOL_WINDOW(10186)     :18
   ——                                  :19
   直播/特效/电视/DVD/滤镜/比例/尺寸/裁剪/皮肤/其他 子菜单  :20–29
CMD_POPUPMENU_PLAY(21012) ▶            :31
CMD_POPUPMENU_CAPTION ▶                :32
CMD_POPUPMENU_VIDEO(21030) ▶           :33
CMD_POPUPMENU_AUDIO ▶                  :34
$MENU_HELP ▶（主页 10386 / 检查更新 10392 / —— / 关于 57664） :35–40
——                                     :41
ID_PLAYINFO_WINDOW(10195)              :42
ID_OPTION_WINDOW(10010)                :43
——                                     :44
ID_LOGIN_MENU(10187)                   :45
ID_APP_EXIT(57665)                     :46
```

数字由符号表恢复（`$P/workB/symtab.json`，注册点地址见 §1 第 3 条）。该文件说明菜单可由 XML 按符号名重组，“视图”分组是 PotPlayer 自己的做法：**播放列表、控制面板、直播列表、聊天、直播工具这五个窗口开关放在同一个视图子菜单里**。选项 → 皮肤设置页 Dialog(376) 有 “Open Menu Folder / Context Menu Items”（L1022 段），字符串 4742 `Context Menu Style`/`菜单输出方式`、4743 `Context Menu Design`/`弹出菜单类型`（中文 ini 5545–5546），说明菜单样式可切换；默认值静态未知（**U2**）。

### 3.5 勾选/单选状态（MFC `ON_UPDATE_COMMAND_UI`，推断）

方法：在 `.rdata` 中扫描 MFC 消息映射条目 `{WM_COMMAND=0x111, CN_UPDATE_COMMAND_UI=0xFFFFFFFF, nID, nLastID, sig, pfn}`，共 340 条（`$P/workB/msgmap.json`），再反汇编处理函数：`jmp/call [reg+8]` = `CCmdUI::SetCheck`；跳到 `0x18040f160` = 单选（该函数先 `SetCheck`，然后 `GetMenuItemInfoW(fMask=0x100 MIIM_FTYPE)`、`fType |= 0x200 MFT_RADIOCHECK`、`SetMenuItemInfoW`）。

| CmdID | 项 | 状态类型 | 处理函数 | 说明 |
| --- | --- | --- | --- | --- |
| 10011 | 列表... (播放列表) | 复选 | `0x1803c97e0` | `SetCheck(byte [this+0x350])`，并检查窗口 `GWL_STYLE`（`GetWindowLongW(-16)`） |
| 10383 | 控制... (控制面板) | 复选 | `0x1803c8140` | 控制窗口对象 `[this+0x4f08]` 存在且可见时 `SetCheck(1)` |
| 10013 | 全屏 | 复选 | `0x1803a5350` | |
| 10037 | 静音 | 复选 | `0x1803c5bb0` | |
| 10095 | 均衡器 | 复选 | `0x1803c5b70` | |
| 10126 | 显示字幕 | 复选 | `0x1803c7740` | |
| 10257 / 10256 | 软模糊 / 锐化 | 复选 | `0x1803cbc10` / `0x1803cc350` | |
| 10369 | 视频下自动隐藏 | 复选 | `0x1803cb020` | |
| 10442 | 播放信息(OSD) | 复选 | `0x1803ca470` | |
| 10085–10093 | 图像属性复位与亮度等 | 复选（范围处理） | `0x1803cc050` | 推断仅“复位”项会被勾上 |
| 10246–10248 | 播放速度 | 复选 | `0x1803c9e50` | 10246 在 `speed == 1.0`（常量 `0x181330150`，字段 `[this+0x698]`）时打勾 |
| 10396–10398 | 变速方式 | **单选** | `0x1803c60b0` | |
| 10495–10497 | 循环方式 | **单选** | `0x1803ca1a0` | |
| 10354–10356 | BMP/JPEG/PNG | **单选** | `0x1803cbce0` | |
| 10038–10039 | 置顶方式 | **单选** | `0x1803cb8d0` | |
| 10351 | OSD 信息 | 单选/复选 | `0x1803ca490` | 两条分支 |
| 10325 / 10222 / 10370 / 10228 / 10195 | 手动定位 / 连续截图 / 录制视频 / 字幕浏览器 / 属性 | 复选（窗口打开时） | `0x1803c9050` / `0x1803cbb90` / `0x1803cabb0` / `0x1803c7990` / `0x1803ca4b0` | 窗口指针（如 `[this+0x7590]`）非空 → `SetCheck(1)` |
| 10059–10066 | 步退/步进 %s | 动态文字 | `0x1803ca0f0` | 按 `(id-0x274B)/2` 从 `[this+0x182c]` 取秒数，调用 `0x180400e40` 填入 `%s` |
| 10014 | 播放/暂停 | 无更新处理 | — | 普通命令 |

### 3.6 加速键（RT_ACCELERATOR）

- 表 157（主窗口，文件偏移 28726664，259 条）：`Space→10014`，`Left→10059`，`Right→10060`，`G→10325`，`F3→10443`，`Ctrl+O→10158`，`Ctrl+U→10159`，`F4→10167`，`F5→10010`，`F6→10011`，`F7→10383`，`Ctrl+F1→10195`，`M→10037`，`Up/Down→10035/10036`，`Z/X/C→10246/10247/10248`，`Shift+E→10095`，`Alt+O→10146`，`Alt+H→10126`，`Alt+E→10228`，`Ctrl+Alt+Y→10488`，`Ctrl+E→10224`，`Ctrl+G→10222`，`Alt+C→10370`，`Ctrl+B→10257`，`Q/W/E/R/T/Y/U/I/O→10085…10093`，`Tab→10351`，`Shift+Tab→10449`，`ScrollLock→10442`，`PgUp/PgDn→10123/10124`。
- 表 118（播放列表窗口，文件偏移 28728736，54 条）：`Enter`/`P→10199` 播放，`Del→10200`，`Ctrl+Del→10212`，`Alt+Del→10201`，`Ctrl+I→10196`，`Ctrl+O→10197`，`Ctrl+U→10198`，**`Ctrl+Up→27020 CMD_PLAYLIST_UP`，`Ctrl+Down→27021 CMD_PLAYLIST_DOWN`，`Ctrl+Home→27022 CMD_PLAYLIST_TOP`，`Ctrl+End→27023 CMD_PLAYLIST_BOTTOM`**，`Ctrl+PgUp/PgDn→27026/27027`，`F6→10446`（关闭列表），`F5→10315`（列表设置）。
- 播放列表“移动所选项”在静态 PL 菜单中没有，但字符串表有 4717–4721、4727–4728（`English.ini:5542-5552`，如 `4718=Move selected items up \tCtrl+Up`），推断是运行时动态插入的菜单项（**推断**）。

## 4. 播放列表与控制面板的显示切换

### 4.1 播放列表 `ID_PLAYLIST_WINDOW`（10011）

- 入口：主菜单顶层 `列表... F6`（`101_0_24`，L3244）；SimpleMenu 视图子菜单首项（`SimpleMenu.xml:14`）；底栏按钮（`BottomVideo.txt:64`：`CmdID='ID_PLAYLIST_WINDOW' DownFlag='1' Width='23'`，图 23×22，`VisMinLeft='320'`）；列表窗口自身标题栏关闭按钮（`SubCommon.txt:511/573`：`Tooltip='$CMD_CLOSE_WINDOW' CmdID='ID_PLAYLIST_WINDOW' Width='14'`）。
- 快捷键：主窗口 F6 → 10011（加速键表 157）；列表窗口获得焦点时 F6 → 10446“关闭列表”（表 118，`111_0_17`，L3343）。
- 状态：复选（§3.5）；底栏按钮 `DownFlag='1'` 表示按钮随状态保持按下（**推断**）。
- 工具提示：`$ID_PLAYLIST_WINDOW`，各语言为 “Playlist On/Off / 显示/隐藏播放列表”一类（`StringTable.txt:323,387,604…`）。
- 列表窗口皮肤：`PlaylistSkinArea Image='PlayList.png' WidthLimit='300' HeightLimit='250'`（`SubCommon.txt:549`），最小 300×250 px（细节归皮肤分析）。

### 4.2 控制面板 `ID_CONTROL_BOX_WINDOW`（10383）

- 入口：顶层 `控制... F7`（`101_0_25`，L3245）；SimpleMenu 视图子菜单第二项（`:15`）；底栏按钮（`BottomVideo.txt:62`，同为 23×22、`DownFlag='1'`）；控制窗口标题栏关闭按钮（`SubCommon.txt:196`）。
- 状态：复选（§3.5）。另有 `CMD_CONTROL_BOX_WINDOW_ON/OFF`（26012/26013，`symtab.json`）。
- **它是独立浮动窗口**，不是底部播放栏。非触屏皮肤规格（`SubCommon.txt:189-369`）：

```css
.control-box {                      /* ControlBoxSkinArea, SubCommon.txt:189 */
  width: 301px; height: 227px;      /* UseWindowFrame='0'：无系统边框，皮肤自绘 */
}
.control-box .titlebar {            /* :191 */
  height: 34px; padding-inline: 10px;
  /* 左：标题 $CONTROL_BOX_TITLE（“控制窗口”），#DEFAULT_UI_FONT 11px bold，颜色取 Control.png(482,42) = #f0f0f0 */
  /* 右→左：关闭 14×13（CmdID=ID_CONTROL_BOX_WINDOW）、10px、置顶钉 14×13（CMD_TOPMOST_WINDOW 20066）、10px、透明度滑块 50px（CMD_TRANS_CB 24006） */
}
.control-box .border-left, .border-right { width: 3px; }   /* :206/:208 */
.control-box .border-bottom { height: 3px; }               /* :204 */
.control-box .tabs {                /* :211，高 31px，四个页签各 74×31 */
  /* 顺序：音频 | 视频 | 字幕 | 播放（触屏皮肤 :24-35 的顺序为 字幕 | 视频 | 播放 | 音频） */
  font: 13px var(--default-ui-font);
  color: #777777;                   /* 未选，Control.png(482,58) */
}
.control-box .tabs .hover, .tabs .active { color: #f0f0f0; } /* Over/Push = (482,42) */
.control-box .page { width: 295px; height: 159px; top: 31px; }  /* :228 等 */
.control-box .label { font-size: 11px; color: #f0f0f0; text-align: right; } /* (482,74) */
.control-box .button-text { color: #aeaeae; }               /* (482,89) */
```

页面内容（坐标为页内 px）：
- 音频页（:228-255）：均衡器开关 `ID_AUDIO_EQULIZER` 82×14 @ (10,24)；预设名显示按钮 `CMD_POPUPMENU_AUDIO_EQULIZER` 97×22 @ (90,19)；“预置”按钮 `CMD_POPUPMENU_EQULIZER_PRESET` 41×22 @ (192,19)；“音量”标签 44×20 @ (240,20)；10 条竖向 EQ 滑块 `CMD_EQULIZER_0…9`，每条 14×70，x = 11 + 23n，y = 69，滑块头 14×12；主音量 `CMD_MASTER_VOLUME` @ (247,69)、波形音量 `CMD_WAVE_VOLUME` @ (265,69)，同为 14×70。
- 视频页（:256-296）：四行“标签 65×20（右对齐）+ 横向滑块 154×12（滑块头 14×12）+ 默认按钮 41×18（字号 10）”，行 y = 21/45/69/93（行距 24px），对应 `CMD_COLOR_OFFSET/GAIN/SAT/HUE`（亮度/对比度/饱和度/色调，24010–24013）及 `_DEFAULT`（20053–20056）；底部三个开关 51×13：软模糊 `ID_VIDEO_BLUR` 10257、锐化 10256、去块 10218，y=133；“截图”按钮 `ID_VIDEO_IMAGE_CAPTURE_PROMPT` 10357，41×22 @ (235,129)。
- 字幕页（:297-333）：字体选择按钮 160×22、字号显示与 ± 按钮 16×11/16×12；位置“上/下/左/右” 4 个 51×22（x=64/118/172/226，y=67，间距 3px）；同步“慢/快/默认” 3 个 69×22（x=64/136/208，y=118，间距 3px），对应 10141/10140/10142。
- 播放页（:334-368）：跳转 4 个 51×22（`ID_PLAYBACK_1MIN_BACK`/`5SEC_BACK`/`5SEC_FORW`/`1MIN_FORW`，AltCmdID 为 5MIN/30SEC），y=18；速度“慢/快/默认” 3 个 69×22（10247/10248/10246），y=66；AB 循环行 y=114。

### 4.3 底部控制栏的显示/隐藏

PotPlayer 静态菜单、SimpleMenu、加速键表中都没有“显示/隐藏底部控制栏”的命令。相关的只有：`皮肤 → 视频下自动隐藏`（10369 `ID_SKIN_AUTO_HIDE`，复选，L3107）、`全屏时不遮盖画面`（10340）、`以视频皮肤启动`（10402）、`不切换界面模式`（10578）。控制栏在全屏/窗口下的自动隐藏行为属于皮肤和运行时逻辑（**U3**）。

## 5. 我们的 46 条命令 → PotPlayer 对应项

“路径”用 PotPlayer 中文菜单文字（主菜单 101，除注明 PL = 播放列表菜单 111）。“键”是加速键表实际绑定（§3.6）；只有菜单文字标注、加速键表没有的写“（仅标注）”。可信度：直接对应 = **确认**；含义相近 = **推断**。

| # | 我们的命令 | PotPlayer 路径 | CmdID / 符号 | 键 | 可信度 |
| --- | --- | --- | --- | --- | --- |
| 1 | 文件 → 打开媒体… | 顶层 打开文件...；或 打开 → 打开文件... | 10443 `ID_BASE_OPEN`；10158 `ID_OPEN_FILE` | F3；Ctrl+O | 确认 |
| 2 | 文件 → 关闭当前来源 | 顶层 关闭 | 10167 `ID_CLOSE_MEDIA` | F4 | 确认 |
| 3 | 播放列表 → 添加文件… | 播放 → 播放列表 → 添加/编辑 → 添加文件...；PL 添加&&编辑 → 添加文件... | 10196 `ID_PLAYLIST_FILE_ADD` | 列表内 Ctrl+I | 确认 |
| 4 | 播放列表 → 添加链接… | 同上 → 添加链接... | 10198 `ID_PLAYLIST_URL_ADD` | 列表内 Ctrl+U | 确认 |
| 5 | 播放列表 → 播放选中项 | PL 播放（`111_0_0`） | 10199 `ID_PLAYLIST_PLAY` | 列表内 Enter / P | 确认 |
| 6 | 播放列表 → 上一项 | 播放 → 上一文件 | 10067 `ID_PLAYBACK_PREV` | PgUp（加速键实际绑 10123 `ID_PROCESS_PREV`） | 推断 |
| 7 | 播放列表 → 下一项 | 播放 → 下一文件 | 10068 `ID_PLAYBACK_NEXT` | PgDn（实际绑 10124 `ID_PROCESS_NEXT`） | 推断 |
| 8 | 播放列表 → 移除选中项 | 播放列表 → 删除 → 移除所选名单；PL 移除 → 移除所选名单 | 10200 `ID_PLAYLIST_DEL` | 列表内 Del | 确认 |
| 9 | 播放列表 → 上移 | 无静态菜单项；字符串 4718 | 27020 `CMD_PLAYLIST_UP` | 列表内 Ctrl+Up | 推断 |
| 10 | 播放列表 → 下移 | 无静态菜单项；字符串 4719 | 27021 `CMD_PLAYLIST_DOWN` | 列表内 Ctrl+Down | 推断 |
| 11 | 播放列表 → 清空列表 | 删除 → 移除所有名单 | 10201 `ID_PLAYLIST_DEL_ALL` | 列表内 Alt+Del | 确认 |
| 12 | 网络媒体 → YouTube 视频… | 无专门项；最接近 打开 → 打开链接... | 10159 `ID_OPEN_URL` | Ctrl+U | 推断 |
| 13 | 网络媒体 → Bilibili 视频… | 同上 | 10159 | Ctrl+U | 推断 |
| 14 | 播放 → 播放/暂停 | 播放 → 播放\|暂停 | 10014 `ID_PLAY_PAUSE` | Space | 确认 |
| 15 | 播放 → 跳转到… | 播放 → 定位 → 手动定位...（Dialog 407） | 10325 `ID_GOTO_PLAY` | G | 确认 |
| 16 | 播放 → 后退5秒 | 播放 → 定位 → 步退%s | 10059 `ID_PLAYBACK_5SEC_BACK` | ← | 确认（默认秒数由符号名推断为 5，可在 Dialog 364 改） |
| 17 | 播放 → 前进5秒 | 播放 → 定位 → 步进%s | 10060 `ID_PLAYBACK_5SEC_FORW` | → | 同上 |
| 18 | 速度 → 0.5倍 | 无固定倍率项；只有 减速 - | 10247 `ID_PLAY_SPEED_SLOWER` | X | 推断 |
| 19 | 速度 → 正常速度 | 播放速度 → 正常/之前的速度 | 10246 `ID_PLAY_SPEED_NORMAL` | Z | 确认 |
| 20 | 速度 → 1.5倍 | 无；加速 + | 10248 `ID_PLAY_SPEED_FASTER` | C | 推断 |
| 21 | 速度 → 2倍 | 无；加速 + | 10248 | C | 推断 |
| 22 | 速度 → 自定义… | 无对话框；控制面板播放页速度按钮 | 10246–10248 | | 推断 |
| 23 | 视频 → 画面调节… | 视频 → 图像属性复位 / 亮度±1% / 对比度 / 饱和度 / 色彩度；窗口形式：控制面板视频页（F7）或选项页 Dialog 384（视频 → 视频输出设置... 10363） | 10085–10093；10383；10363 | Q W E R T Y U I O；F7 | 推断 |
| 24 | 视频 → 高斯模糊… | 视频 → 图像处理 → 软模糊（参数在 Dialog 386） | 10257 `ID_VIDEO_BLUR` | Ctrl+B | 推断 |
| 25 | 截图与导出 → 保存当前画面… | 图像截取 → 截取当前源画面为...；截存当前源画面（不弹框） | 10357 `ID_VIDEO_IMAGE_CAPTURE_PROMPT`；10224 `ID_VIDEO_IMAGE_CAPTURE` | —；Ctrl+E | 确认 |
| 26 | 截图与导出 → 连拍… | 图像截取 → 连续截图...（Dialog 337） | 10222 `ID_VIDEO_ADV_CAPTURE` | Ctrl+G | 确认 |
| 27 | 截图与导出 → 导出片段… | 视频录制 → 录制视频...（Dialog 413，实时录制而非区间导出） | 10370 `ID_SAVE_MOVIE` | Alt+C | 推断 |
| 28 | 截图与导出 → 查看导出任务… | 无；最接近 打开截图存档文件夹 / 打开视频录制存档文件夹 | 10225 `ID_VIDEO_OPEN_FOLDER`；10910 | | 推断 |
| 29 | 音频 → 静音 | 声音 → 静音（复选） | 10037 `ID_AUDIO_MUTE` | M | 确认 |
| 30 | 音频 → 音量… | 无音量对话框；声音 → 音量 + / 音量 -；控制面板音频页主音量 | 10035/10036；24024 `CMD_MASTER_VOLUME` | Up / Down | 推断 |
| 31 | 音频 → 音频工作台… | 声音 → 声音输出设置...（打开选项）；控制面板音频页 | 10391 `ID_AUDIO_OUT_SETTING`；10383 | F7 | 推断 |
| 32 | 音频 → 复位音频处理 | 无单一项；均衡器预设菜单 106 → 初始化 | 10184 `ID_EQULIZER_RESET` | | 推断 |
| 33–36 | 均衡器预设 → 平直 / 人声 / 音乐 / 影院 | 声音 → 预置（20008 运行时列表）；声音 → 均衡器（开关） | 20008；10095 `ID_AUDIO_EQULIZER` | Shift+E（开关） | 推断（预设名未知 U6） |
| 37 | 字幕 → 字幕工作台… | 字幕 → 字幕浏览器...；字幕 → 字幕设置... | 10228 `ID_CAPTION_EXPLORER`；10760 `ID_CAPTION_SET` | Alt+E | 推断 |
| 38 | 字幕 → 加载外部 SRT… | 字幕 → 选择字幕 → 打开字幕...；打开 → 打开字幕... | 10146 `ID_CAPTION_OPEN` | Alt+O | 确认 |
| 39 | 字幕 → 关闭字幕 | 字幕 → 显示字幕（复选开关，不是“关闭”） | 10126 `ID_CAPTION_VISIBLE` | Alt+H | 推断 |
| 40 | 字幕 → 刷新字幕轨 | 打开 → 重载字幕 | 10488 `ID_CAPTION_REOPEN` | Ctrl+Alt+Y | 推断 |
| 41 | 视图 → 显示播放栏 | 无等价项；最接近 皮肤 → 视频下自动隐藏（注意 `ID_CONTROL_BOX_WINDOW` 是控制面板） | 10369 `ID_SKIN_AUTO_HIDE` | | 推断 |
| 42 | 视图 → 显示播放列表 | 顶层 列表...；SimpleMenu 视图 → `ID_PLAYLIST_WINDOW` | 10011 `ID_PLAYLIST_WINDOW` | F6 | 确认 |
| 43–45 | 模拟器 → 打开 NES ROM… / NES 工具 / 停止 NES | 无 | — | — | 确认（不存在） |
| 46 | 插件 → 插件工坊… | 无；最接近 选项 → 扩展功能页 Dialog(496)（“Extensions / Check for updates of extensions”） | — | — | 推断 |

与“播放信息/调试显示”插件领域相关、我们没有的项：`配置/语言/其他 → OSD信息 10351 Tab`、`播放信息 10442 ScrollLock`、`简要信息 10449 Shift+Tab`（`101_0_15_16..18`，L3148–L3150），以及顶层 `属性... 10195 Ctrl+F1`（Dialog 424）。

## 6. 对话框规格

对话框与命令的对应关系依据：对话框文字（`inv/dialogs.json`）+ 命令名称；PotPlayer 没有在资源里记录“哪个命令打开哪个对话框”，因此都标 **推断**。所有像素按 §2.1（×1.75）换算。完整控件表见 §6.9 附录。

### 6.1 手动定位 Dialog(407)（10325 G）

```css
.jump-dialog { width: 357px; height: 224px; }          /* 204×128 DLU；WS_POPUP|WS_SYSMENU */
.jump-dialog fieldset { left: 23px; width: 317px; height: 52px; }  /* 13,23,181,30 / 13,58,181,30 DLU，两组垂直间隔 5 DLU = 9px */
.jump-dialog .syslink { width: 89px; height: 19px; }   /* 组内左侧 SysLink（3417/3418，文字运行时设置） */
.jump-dialog input   { left: 124px; width: 140px; height: 21px; }  /* 71,35,80,12 DLU */
.jump-dialog button  { left: 271px; width: 60px; height: 19px; }   /* 155,35,34,11 DLU “定位/Jump” */
/* 组内：链接→输入框间距 4 DLU（7px），输入框→按钮 4 DLU（7px）；底部 y=88 至 128 DLU 留空 40 DLU（70px），推断放运行时按钮或仅为留白 */
```
分组标题：“时间定位(时:分:秒.毫秒)”（L1465）、“按帧移动(帧数,帧率)”（L1466）。ini 段 `[Dialog(407)]` 在 `English.ini:1462`。

### 6.2 打开链接 Dialog(310)（10159 Ctrl+U）

```css
.open-url { width: 464px; height: 522px; }            /* 265×298 DLU；WS_POPUP|CLIP*|DS_CENTER */
.open-url fieldset.main { left: 23px; top: 37px; width: 415px; height: 196px; } /* “打开链接/Open URL” */
.open-url .icon   { left: 40px; top: 58px; width: 35px; height: 35px; }         /* SS_ICON，图标资源 100 */
.open-url .hint   { left: 89px; width: 336px; height: 14px; }                   /* 两行说明 y=56 / 79 px */
.open-url textarea{ left: 32px; top: 100px; width: 397px; height: 68px; }       /* ES_MULTILINE|ES_AUTOVSCROLL|WS_VSCROLL，18,57,227,39 DLU */
.open-url .check  { left: 33px; width: 399px; height: 18px; }                   /* 3 个复选，y=172/191/210 px（行距 11 DLU = 19px） */
.open-url .tabs   { left: 23px; top: 240px; width: 415px; height: 226px; }      /* SysTabControl32（历史列表页签） */
.open-url .list   { /* SysListView32 5,225,234,54 DLU，与 Tab 重叠，推断运行时重新定位 */ }
.open-url .btns   { /* 63×12 DLU = 110×21 px：删除所选 / 打开文件夹 / 删除全部，以及 155 DLU 宽下拉框 */ }
```
注意：模板中按钮与下拉框位置互相重叠（如 3021 @ y=273 与 3022 @ y=286），推断由代码重排（**U8**）。模板没有“确定/取消”。

### 6.3 同步值 Dialog(463) 与帧率转换 Dialog(461)

```css
.sync-dialog { width: 350px; height: 121px; }          /* 200×69 DLU */
.sync-dialog label  { left: 38px; top: 49px; width: 102px; text-align: right; } /* “同步值(毫秒)：” L2051 */
.sync-dialog input  { left: 147px; top: 46px; width: 86px; height: 21px; }      /* + UpDown 18×21 贴右缘 */
.sync-dialog button { left: 240px; top: 44px; width: 75px; height: 23px; }      /* “应用” 43×13 DLU */
/* 标签→输入框 4 DLU(7px)，输入框→按钮 4 DLU(7px)；单行布局，垂直居中于 y≈26–38 DLU */

.fps-dialog { width: 458px; height: 121px; }           /* 262×69 DLU */
/* 源帧率: [下拉 84px] -> 转换后帧率: [下拉 84px]；下拉 48 DLU 宽，"->" 居中 30px */
```

### 6.4 选项框架 Dialog(324)（10010 F5）

```css
.prefs { width: 751px; height: 527px; }                /* 429×301 DLU */
.prefs .search  { left: 16px; top: 37px; width: 205px; } /* 下拉 + 编辑框（9,21/9,36 DLU，宽 117 DLU） */
.prefs .tree    { left: 16px; top: 89px; width: 205px; height: 396px; } /* SysTreeView32 9,51,117,226 DLU */
.prefs .pages   { left: 229px; top: 37px; width: 506px; height: 450px; } /* SysTabControl32 131,21,289,257 DLU；树与页间距 5 DLU = 9px */
.prefs .footer button { height: 24px; }                 /* 14 DLU：初始化 80px / 导出当前配置 178px / 应用 108px，x=248/334/516，间隔 3 DLU = 5px */
/* 右侧 x=357..429 DLU（126px）无控件，推断运行时放“确定/取消/关闭”（U4） */
```
页模板（`WS_CHILD|DS_CONTROL`，exstyle `WS_EX_CONTROLPARENT 0x10000`）尺寸集中在 282–286 × 230–250 DLU（≈494–500 × 403–438 px），与 Tab 客户区匹配。

### 6.5 选项页：图像属性 Dialog(384)、图像处理 Dialog(386)

```css
/* 384：色彩空间组 + 图像属性组 */
.color-page fieldset.colorspace { left: 9px; top: 5px; width: 478px; height: 217px; }  /* 5,3,273,124 DLU */
.color-page fieldset.controls   { left: 9px; top: 229px; width: 478px; height: 161px; } /* 5,131,273,92 DLU “图像属性/Color Controls” */
.color-row {                       /* 亮度/对比度/饱和度/色彩度 四行，y = 143/158/172/186 DLU，行距约 15 DLU = 26px */
  display: grid;
  grid-template-columns: 75px 245px 63px 61px;  /* 标签 43 | 滑块 140 | 数值框 36 | 默认 35 DLU */
  column-gap: 4px;                              /* 2–3 DLU = 3.5–5px */
}
.color-row label  { text-align: right; height: 18px; }  /* SS_RIGHT, 10 DLU */
.color-row .slider{ height: 18px; }                      /* msctls_trackbar32, style 0x50010018 = TBS_BOTH|TBS_NOTICKS */
.color-row input  { height: 21px; }                      /* 只读数值框，style 0x50810880 含 ES_READONLY 0x800 */
.color-row button { height: 21px; }                      /* “默认值” */
/* 组底部：etched 分隔线 y=203 DLU，复选“如可能则使用硬件功能” y=208 DLU */
```

```css
/* 386：图像处理页，每个效果一节，节间用 SS_ETCHEDHORZ 分隔 */
.effects-page .section { padding: 5px 12px; }    /* 复选标题 x=7 DLU(12px)，滑块 x=15 DLU(26px) */
.effects-page .section-title { height: 14px; }   /* autocheck 220×8 DLU，文字含 %d 实时值 */
.effects-page .slider-3 { width: 119px; }        /* 三联滑块 68 DLU，x=15/87/160 DLU，间隔 4–5 DLU */
.effects-page .slider-2 { width: 178px; }        /* 双联 102–103 DLU */
.effects-page .slider-1 { width: 373px; }        /* 单滑块 213 DLU */
.effects-page .default  { left: 402px; width: 66px; height: 21px; }  /* 230,y,38,12 DLU，与滑块垂直对齐（y 比滑块小 2–3 DLU） */
/* 节距约 27–30 DLU（47–53 px）：软模糊 y=31，锐化 58，去块 88，渐进降噪 119，3D降噪 149，瞬态降噪 177，Warpsharp 205，Deband 233 */
```
软模糊（我们的“高斯模糊”最接近项）：`3110` “Soften - Radius : %d  Luma : %d  Chroma : %d”（L1206），三滑块 3359/3360/3361，默认按钮 3166。

### 6.6 选项页：均衡器 Dialog(363)、声音处理 Dialog(387)

```css
.eq-page fieldset { left: 9px; top: 5px; width: 478px; height: 252px; }   /* 5,3,273,144 DLU “均衡器” L739 */
.eq-page .toolbar { top: 21px; }      /* 开启（复选 84px）| 预置（按钮 77×21）| 预设下拉 220px | “0 dB” 按钮 58×21 */
.eq-page .superEQ { left: 38px; top: 44px; }                    /* 复选 SuperEQ */
.eq-band {                             /* 10 段：60 170 310 600 1K 3K 6K 12K 14K 16K（Hz） */
  width: 18px; height: 142px;          /* 竖向 trackbar 10×81 DLU，style 0x5001001a = TBS_VERT|TBS_BOTH|TBS_NOTICKS */
  /* x = 26,74,121,168,214,261,308,355,401,448 px（步进约 27 DLU = 47px） */
  top: 91px;
}
.eq-band .freq  { top: 72px; height: 14px; }   /* 频率标签在滑块上方 11 DLU */
.eq-band .value { top: 231px; height: 14px; text-align: center; } /* 数值标签在滑块下方，初值 "0" */
```

声音处理页 387：两个分组（规格化 5,3,273,110 DLU；混响 Freeverb 5,117,273,112 DLU）；每行“右对齐标签 89 DLU（156px）+ 滑块 134 DLU（234px）+ 默认 38×12 DLU（66×21px）”，行距 16 DLU（28px）。

### 6.7 选项页：字幕字体 Dialog(383)、时间跨度 Dialog(364)

- 383（284×244 DLU）：三个分组——字体（5,3,273,79 DLU：默认/替代字体按钮 124×12 DLU = 217×21 px，字号/字距/行距/注音位置编辑框 36×12 DLU 带 UpDown）；颜色/透明度/边框/阴影（5,84,169,149 DLU：每行 标签 50 + 色块 12×12 DLU（21×21 px，SS 类型 0xD = SS_OWNERDRAW）+ 透明度滑块 60 DLU + 宽度编辑 30 DLU）；位置与边距（179,84,99,121 DLU：段落对齐下拉 + 5 个百分比/边距编辑框，行距 16 DLU）；不透明框（179,207,99,26 DLU：三态复选 + 色块 + 滑块）。
- 364（284×157 DLU）：四行“快捷键说明（右对齐，157 DLU = 275 px）+ 秒数编辑 36×12 DLU + UpDown + ‘秒’ + 默认 43×12 DLU”，行 y = 30/46/62/78 DLU（行距 16 DLU = 28 px）；分别对应 ←/→、Ctrl+←/→、Shift+←/→、Ctrl+Alt+←/→（L761–764），模板中无默认数值。

### 6.8 截图/录制/信息/专辑

```css
/* 连续截图 Dialog(337)，250×281 DLU = 438×492 px */
.capture fieldset.storage  { left: 19px; top: 35px; width: 401px; height: 105px; } /* 路径(348px)+"..."(30×21) / 文件前缀名 / 命名方式下拉 */
.capture fieldset.format   { width: 196px; height: 89px; }  /* 左：格式下拉 + 质量(%)；右：尺寸（原始/自定义 单选 + 宽×高 + "G"） */
.capture fieldset.count    { width: 196px; height: 102px; } /* 采集数量；每毫秒/每帧 单选 + 编辑 */
.capture fieldset.activity { left: 19px; top: 343px; width: 401px; height: 107px; }
.capture .footer button    { height: 24px; }  /* 14 DLU：打开存档文件夹 138px / 截取当前画面 138px / 开始 102px，间隔 2 DLU(3.5px) */
/* 左右两列分组宽 112 DLU，列间距 5 DLU（9px），分组之间垂直间距 2–3 DLU（3.5–5px） */

/* 录制视频 Dialog(413)，274×323 DLU = 480×565 px */
/* 存储（12,20,252,76）/ 输出设置（12,100,252,131）/ 信息（12,235,252,57）三组，组间 4 DLU；
   表单左列标签 57 DLU（100px）右对齐，控件起点 x=77 DLU（135px）；底部 2 个按钮 121×13 与 118×13 DLU（212×23 / 206×23 px） */

/* 播放/系统信息 Dialog(424)，287×309 DLU = 502×541 px */
/* 顶部一行 5 个“页签式”按钮，高 14 DLU（24.5px）：播放信息 88px / 媒体内嵌资源 138px / 文件信息 74px / 系统信息 82px / 在线信息 82px，间隔 3–4 DLU；
   下面 Tab 控件 7,37,208,171 DLU（推断运行时撑满）；底部“复制到剪贴板”“保存为...” 95×14 DLU（166×24 px） */

/* 专辑编辑 Dialog(403)，444×221 DLU = 777×387 px */
/* ListView 15,24,415,156 DLU（726×273 px）；下方一排 9 个按钮，41×11 DLU（72×19 px），“添加到播放列表”71×11 DLU（124×19 px），间隔 2 DLU（3.5px） */
```

### 6.9 附录：对话框控件表

列说明：类型（Button/push、autocheck、autoradio、groupbox；Static/left|center|right|etched-h；ComboBox/droplist|dropdown；Trackbar/horz|vert）；id 为控件 ID（-1 = IDC_STATIC）；文本取自 `inv/dialogs.json`（L = `English.ini` 行号）；px 按 ×1.75 四舍五入。语言文件段起始行：`[Dialog(310)]` 104，`(324)` 199，`(337)` 340，`(363)` 722，`(364)` 749，`(383)` 1130，`(384)` 1168，`(386)` 1204，`(387)` 1228，`(403)` 1425，`(407)` 1462，`(412)` 1514，`(413)` 1518，`(424)` 1641，`(461)` 2031，`(463)` 2049。

#### Dialog(407) — 手动定位（推断对应 ID_GOTO_PLAY 10325）

模板：`204×128` DLU ≈ `357×224` px；字体 Arial 9pt；style `0x80080040`；资源 RT_DIALOG 407，文件偏移 28801984（420 字节）；控件 8 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| Button/groupbox | 3381 | Jump to specified time (hh:mm:ss.ms or sec): / 时间定位(时:分:秒.毫秒) (L1465) | 13,23,181,30 | 23,40,317,52 |
| SysLink | 3417 |  | 16,36,51,11 | 28,63,89,19 |
| Edit | 3069 |  | 71,35,80,12 | 124,61,140,21 |
| Button/push | 3021 | Jump / 定位 (L1463) | 155,35,34,11 | 271,61,60,19 |
| Button/groupbox | 3382 | Jump to specified frame (frame number, fps): / 按帧移动(帧数,帧率) (L1466) | 13,58,181,30 | 23,102,317,52 |
| SysLink | 3418 |  | 17,70,51,11 | 30,122,89,19 |
| Edit | 3077 |  | 71,69,80,12 | 124,121,140,21 |
| Button/push | 3024 | Jump / 定位 (L1464) | 155,69,34,11 | 271,121,60,19 |

#### Dialog(310) — 打开链接（推断对应 ID_OPEN_URL 10159）

模板：`265×298` DLU ≈ `464×522` px；字体 Arial 9pt；style `0x86000840`；资源 RT_DIALOG 310，文件偏移 28740840（812 字节）；控件 14 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| Button/groupbox | 3381 | Open URL / 打开链接 (L113) | 13,21,237,112 | 23,37,415,196 |
| Static/icon | -1 |  | 23,33,20,20 | 40,58,35,35 |
| Static/left | 3285 | Input the URL of the stream or file to be played / 在下方输入您需要播放的文件网址。 (L111) | 51,32,192,8 | 89,56,336,14 |
| Static/left | 3286 | For example, https://www.kakao.com/test.mp3 / 例) https://www.kakao.com (L112) | 51,45,191,8 | 89,79,334,14 |
| Edit | 3069 |  | 18,57,227,39 | 32,100,397,68 |
| Button/autocheck | 3036 | Store only entered URLs (prevents storing URLs p / 保存用户输入的地址到列表中 (L110) | 19,98,228,10 | 33,172,399,18 |
| Button/autocheck | 3034 | Auto paste URLs from clipboard / 默认使用剪贴板中可用地址 (L108) | 19,109,228,10 | 33,191,399,18 |
| Button/autocheck | 3035 | Process subtitles / 同时打开字幕 (L109) | 19,120,228,10 | 33,210,399,18 |
| Tab | 3380 |  | 13,137,237,129 | 23,240,415,226 |
| ComboBox/droplist | 3005 |  | 8,286,155,140 | 14,500,271,245 |
| Button/push | 3021 | Open file location / 打开文件夹 (L105) | 194,273,63,12 | 340,478,110,21 |
| Button/push | 3023 | Delete selected / 删除所选 (L107) | 133,273,63,12 | 233,478,110,21 |
| Button/push | 3022 | Clear all / 删除全部 (L106) | 185,286,63,12 | 324,500,110,21 |
| ListView | 3242 |  | 5,225,234,54 | 9,394,410,94 |

#### Dialog(463) — 同步值输入（推断对应 ID_CAPTION_PROMPT_SYNC 10865 / ID_AUDIO_PROMPT_SYNC 10878）

模板：`200×69` DLU ≈ `350×121` px；字体 Arial 9pt；style `0x80000040`；资源 RT_DIALOG 463，文件偏移 28874480（232 字节）；控件 4 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| Static/right | 3381 | Sync value (ms): / 同步值(毫秒)： (L2051) | 22,28,58,8 | 38,49,102,14 |
| Edit | 3069 |  | 84,26,49,12 | 147,46,86,21 |
| UpDown | 3259 |  | 127,32,10,12 | 222,56,18,21 |
| Button/push | 3021 | Apply / 应用 (L2050) | 137,25,43,13 | 240,44,75,23 |

#### Dialog(461) — 字幕帧率转换（推断对应 ID_CAPTION_FPS_CONV_USER 10851）

模板：`262×69` DLU ≈ `458×121` px；字体 Arial 9pt；style `0x80000040`；资源 RT_DIALOG 461，文件偏移 28873584（256 字节）；控件 5 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| Static/right | 3381 | Input FPS :  / 源帧率： (L2033) | 22,28,46,8 | 38,49,80,14 |
| ComboBox/dropdown | 3003 |  | 70,26,48,51 | 122,46,84,89 |
| Static/right | 3382 | Output FPS :  / 转换后帧率： (L2034) | 141,28,46,8 | 247,49,80,14 |
| ComboBox/dropdown | 3004 |  | 189,26,48,51 | 331,46,84,89 |
| Static/center | 3204 | -> / -> (L2032) | 121,28,17,8 | 212,49,30,14 |

#### Dialog(412) — 自定义窗口尺寸（推断对应 ID_SCREEN_SIZE_USER 10404）

模板：`208×82` DLU ≈ `364×144` px；字体 Arial 9pt；style `0x80000040`；资源 RT_DIALOG 412，文件偏移 28805504（316 字节）；控件 6 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| Static/right | 3381 | Width: / 横向： (L1515) | 17,34,31,8 | 30,60,54,14 |
| Edit | 3069 |  | 51,32,50,12 | 89,56,88,21 |
| UpDown | 3248 |  | 94,42,10,12 | 164,74,18,21 |
| Static/right | 3382 | Height: / 纵向： (L1516) | 105,34,31,8 | 184,60,54,14 |
| Edit | 3077 |  | 138,32,50,12 | 242,56,88,21 |
| UpDown | 3249 |  | 181,43,10,12 | 317,75,18,21 |

#### Dialog(384) — 选项页：视频 → 色彩空间 / 图像属性

模板：`286×234` DLU ≈ `500×410` px；字体 Arial 9pt；style `0x40000440`；资源 RT_DIALOG 384，文件偏移 28828000（1652 字节）；控件 33 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| Button/groupbox | 3296 | Supported Colorspaces / YCbCr <-> RGB conversion / 色彩空间 (L1177) | 5,3,273,124 | 9,5,478,217 |
| Static/left | 3297 | Here you can specify the color space of the vide / 可以选择应用于视频输出的色彩空间 (L1178) | 12,13,262,10 | 21,23,458,18 |
| Static/right | 3306 | Output color space: / 输出色彩空间： (L1182) | 10,26,89,10 | 18,46,156,18 |
| ComboBox/droplist | 3100 |  | 102,25,170,137 | 178,44,298,240 |
| Static/right | 3307 | Direct conversion: / 如有可能则直接转换： (L1183) | 10,41,89,10 | 18,72,156,18 |
| ComboBox/droplist | 3101 |  | 102,40,170,137 | 178,70,298,240 |
| Button/autocheck | 3139 | Multi-threaded conversion / 使用多核处理色彩空间输出转换 (L1175) | 103,56,170,9 | 180,98,298,16 |
| Button/autocheck | 3138 | High quality YUY2/RGB24/32 conversion / YUY2\|RGB24/32 高画质转换 (L1174) | 103,66,170,9 | 180,116,298,16 |
| Static/right | 3301 | YCbCr <-> RGB conv. matrix: / YCbCr<->RGB 规则： (L1179) | 10,81,89,10 | 18,142,156,18 |
| ComboBox/droplist | 3168 |  | 102,79,170,137 | 178,138,298,240 |
| Static/right | 3302 | YCbCr processing area: / YCbCr 处理区域： (L1180) | 10,96,89,10 | 18,168,156,18 |
| ComboBox/droplist | 3176 |  | 102,94,170,137 | 178,164,298,240 |
| Static/right | 3304 | RGB processing area: / RGB 处理区域： (L1181) | 10,111,89,10 | 18,194,156,18 |
| ComboBox/droplist | 3180 |  | 102,109,170,137 | 178,191,298,240 |
| Button/groupbox | 3289 | Color Controls / 图像属性 (L1176) | 5,131,273,92 | 9,229,478,161 |
| Static/right | 3308 | Brightness: / 亮度： (L1184) | 11,143,43,10 | 19,250,75,18 |
| Trackbar/horz | 3045 |  | 56,143,140,10 | 98,250,245,18 |
| Edit | 3322 |  | 199,140,36,12 | 348,245,63,21 |
| Button/push | 3029 | Default / 默认值 (L1170) | 238,140,35,12 | 416,245,61,21 |
| Static/right | 3310 | Contrast: / 对比度： (L1185) | 11,158,43,10 | 19,276,75,18 |
| Trackbar/horz | 3043 |  | 56,158,140,10 | 98,276,245,18 |
| Edit | 3346 |  | 199,155,36,12 | 348,271,63,21 |
| Button/push | 3027 | Default / 默认值 (L1169) | 238,155,35,12 | 416,271,61,21 |
| Static/right | 3312 | Saturation: / 饱和度： (L1186) | 11,173,43,10 | 19,303,75,18 |
| Trackbar/horz | 3058 |  | 56,172,140,10 | 98,301,245,18 |
| Edit | 3347 |  | 199,170,36,12 | 348,298,63,21 |
| Button/push | 3033 | Default / 默认值 (L1172) | 238,170,35,12 | 416,298,61,21 |
| Static/right | 3314 | Hue: / 色彩度： (L1187) | 11,186,43,10 | 19,326,75,18 |
| Trackbar/horz | 3053 |  | 56,186,140,10 | 98,326,245,18 |
| Edit | 3348 |  | 199,185,36,12 | 348,324,63,21 |
| Button/push | 3031 | Default / 默认值 (L1171) | 238,185,35,12 | 416,324,61,21 |
| Button/autocheck | 3083 | Use hardware color controls (if possible) / 如可能则使用硬件(H/W)功能来处理 (L1173) | 13,208,184,10 | 23,364,322,18 |
| Static/etched-h | 3237 |  | 8,203,269,2 | 14,355,471,4 |

#### Dialog(386) — 选项页：图像处理（模糊/锐化/降噪）

模板：`286×266` DLU ≈ `500×466` px；字体 Arial 9pt；style `0x40000440`；资源 RT_DIALOG 386，文件偏移 28830720（2650 字节）；控件 47 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| Button/autocheck | 3061 | Flip Vertical / 上下翻转 (L1205) | 7,3,74,10 | 12,5,130,18 |
| Button/autocheck | 3372 | 270 degrees / 旋转 (L1223) | 86,3,73,10 | 150,5,128,18 |
| Button/autocheck | 3374 | Multi-threaded video processing / 利用多核进行图像处理 (L1225) | 165,2,107,10 | 289,4,187,18 |
| Button/autocheck | 3149 | Flip Horizontal / 左右翻转 (L1212) | 7,14,74,10 | 12,24,130,18 |
| Button/autocheck | 3373 | Motion Blur / 倍帧(创建中间帧) (L1224) | 86,14,185,10 | 150,24,324,18 |
| Static/etched-h | -1 |  | 3,26,269,1 | 5,46,471,2 |
| Button/autocheck | 3110 | Soften - Radius : %d Luma : %d Chroma : %d / 软模糊 - 半径：%d 对比度：%d 饱和度：% (L1206) | 7,31,220,8 | 12,54,385,14 |
| Trackbar/horz | 3359 |  | 15,41,68,10 | 26,72,119,18 |
| Trackbar/horz | 3360 |  | 87,41,68,10 | 152,72,119,18 |
| Trackbar/horz | 3361 |  | 160,41,68,10 | 280,72,119,18 |
| Button/push | 3166 | Default / 默认值 (L1213) | 230,38,38,12 | 402,66,66,21 |
| Static/etched-h | -1 |  | 3,53,269,1 | 5,93,471,2 |
| Button/autocheck | 3115 | Sharpen - Luma : %d Chroma : %d / 锐化 - 对比度：%d 鲜明度：%d (L1207) | 7,58,220,8 | 12,102,385,14 |
| Trackbar/horz | 3084 |  | 15,70,102,10 | 26,122,178,18 |
| Trackbar/horz | 3353 |  | 124,70,103,10 | 217,122,180,18 |
| Button/push | 3171 | Default / 默认值 (L1214) | 230,68,38,12 | 402,119,66,21 |
| Static/etched-h | -1 |  | 4,83,269,1 | 7,145,471,2 |
| Button/autocheck | 3124 | Deblock : %d / 去块：%d (L1209) | 7,88,220,8 | 12,154,385,14 |
| Trackbar/horz | 3231 |  | 15,100,213,10 | 26,175,373,18 |
| Button/push | 3320 | Default / 默认值 (L1216) | 230,98,38,12 | 402,172,66,21 |
| Static/etched-h | -1 |  | 4,113,269,1 | 7,198,471,2 |
| Button/autocheck | 3138 | Gradual Denoise : %d / 渐进降噪：%d (L1211) | 7,119,220,8 | 12,208,385,14 |
| Trackbar/horz | 3223 |  | 15,131,213,10 | 26,229,373,18 |
| Button/push | 3121 | Default / 默认值 (L1208) | 231,129,38,12 | 404,226,66,21 |
| Static/etched-h | -1 |  | 4,144,269,1 | 7,252,471,2 |
| Button/autocheck | 3133 | Denoise 3D - Luma : %d Chroma : %d Time : %d / 3D降噪 - 对比度：%d 饱和度：%d 时间： (L1210) | 7,149,204,8 | 12,261,357,14 |
| Button/autocheck | 3332 | High quality / 高质量处理 (L1217) | 213,148,53,10 | 373,259,93,18 |
| Trackbar/horz | 3098 |  | 15,162,68,10 | 26,284,119,18 |
| Trackbar/horz | 3103 |  | 87,162,68,10 | 152,284,119,18 |
| Trackbar/horz | 3362 |  | 160,162,68,10 | 280,284,119,18 |
| Button/push | 3371 | Default / 默认值 (L1222) | 231,159,38,12 | 404,278,66,21 |
| Static/etched-h | -1 |  | 4,174,269,1 | 7,304,471,2 |
| Button/autocheck | 3363 | Temporal noise reducer : %d %d %d / 瞬态降噪：%d %d %d (L1218) | 7,177,220,8 | 12,310,385,14 |
| Trackbar/horz | 3364 |  | 15,189,68,10 | 26,331,119,18 |
| Trackbar/horz | 3227 |  | 87,189,68,10 | 152,331,119,18 |
| Trackbar/horz | 3365 |  | 160,189,68,10 | 280,331,119,18 |
| Button/push | 3184 | Default / 默认值 (L1215) | 231,186,38,12 | 404,326,66,21 |
| Static/etched-h | -1 |  | 4,201,269,1 | 7,352,471,2 |
| Button/autocheck | 3366 | Warpsharp - Depth : %d Threshold : %d / 边缘锐化 - 级别：%d 阀值：%d (L1219) | 7,205,220,8 | 12,359,385,14 |
| Trackbar/horz | 3367 |  | 15,216,102,10 | 26,378,178,18 |
| Trackbar/horz | 3368 |  | 124,216,103,10 | 217,378,180,18 |
| Button/push | 3369 | Default / 默认值 (L1220) | 230,215,38,12 | 402,376,66,21 |
| Static/etched-h | -1 |  | 4,230,269,1 | 7,402,471,2 |
| Button/autocheck | 3370 | Deband - Threshold : %g Radius : %d / 消色块 - 边界值：%g 半径：%d (L1221) | 7,233,220,8 | 12,408,385,14 |
| Trackbar/horz | 3375 |  | 15,244,102,10 | 26,427,178,18 |
| Trackbar/horz | 3376 |  | 124,244,103,10 | 217,427,180,18 |
| Button/push | 3377 | Default / 默认值 (L1226) | 230,243,38,12 | 402,425,66,21 |

#### Dialog(363) — 选项页：均衡器

模板：`282×155` DLU ≈ `494×271` px；字体 Arial 9pt；style `0x40000440`；资源 RT_DIALOG 363，文件偏移 28778536（1678 字节）；控件 36 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| Button/groupbox | 3287 | Equalizer / 均衡器 (L739) | 5,3,273,144 | 9,5,478,252 |
| Button/autocheck | 3066 | Enable / 开启 (L725) | 11,14,48,10 | 19,24,84,18 |
| Button/push | 3021 | Preset / 预置 (L723) | 63,12,44,12 | 110,21,77,21 |
| ComboBox/droplist | 3005 |  | 111,12,126,179 | 194,21,220,313 |
| Button/push | 3024 | 0 dB / 0 dB (L724) | 240,12,33,12 | 420,21,58,21 |
| Button/autocheck | 3072 | SuperEQ / SuperEQ 均衡 (L726) | 22,25,61,10 | 38,44,107,18 |
| Static/left | 3381 | 60 / 60 (L740) | 15,41,9,8 | 26,72,16,14 |
| Static/left | 3382 | 170 / 170 (L741) | 39,41,13,8 | 68,72,23,14 |
| Static/left | 3383 | 310 / 310 (L742) | 67,41,13,8 | 117,72,23,14 |
| Static/left | 3384 | 600 / 600 (L743) | 95,41,13,8 | 166,72,23,14 |
| Static/left | 3196 | 1K / 1K (L732) | 122,41,10,8 | 214,72,18,14 |
| Static/left | 3187 | 3K / 3K (L728) | 149,41,10,8 | 261,72,18,14 |
| Static/left | 3199 | 6K / 6K (L733) | 175,41,10,8 | 306,72,18,14 |
| Static/left | 3185 | 12K / 12K (L727) | 200,41,14,8 | 350,72,24,14 |
| Static/left | 3201 | 14K / 14K (L734) | 226,41,14,8 | 396,72,24,14 |
| Static/left | 3191 | 16K / 16K (L730) | 254,41,14,8 | 444,72,24,14 |
| Trackbar/vert | 3043 |  | 15,52,10,81 | 26,91,18,142 |
| Trackbar/vert | 3045 |  | 42,52,10,81 | 74,91,18,142 |
| Trackbar/vert | 3053 |  | 69,52,10,81 | 121,91,18,142 |
| Trackbar/vert | 3058 |  | 96,52,10,81 | 168,91,18,142 |
| Trackbar/vert | 3062 |  | 122,52,10,81 | 214,91,18,142 |
| Trackbar/vert | 3093 |  | 149,52,10,81 | 261,91,18,142 |
| Trackbar/vert | 3098 |  | 176,52,10,81 | 308,91,18,142 |
| Trackbar/vert | 3103 |  | 203,52,10,81 | 355,91,18,142 |
| Trackbar/vert | 3108 |  | 229,52,10,81 | 401,91,18,142 |
| Trackbar/vert | 3112 |  | 256,52,10,81 | 448,91,18,142 |
| Static/center | 3387 | 0 / 0 (L744) | 10,132,17,8 | 18,231,30,14 |
| Static/center | 3388 | 0 / 0 (L745) | 38,132,15,8 | 66,231,26,14 |
| Static/center | 3389 | 0 / 0 (L746) | 66,132,13,8 | 116,231,23,14 |
| Static/center | 3391 | 0 / 0 (L747) | 94,132,13,8 | 164,231,23,14 |
| Static/center | 3204 | 0 / 0 (L735) | 120,132,12,8 | 210,231,21,14 |
| Static/center | 3189 | 0 / 0 (L729) | 145,132,16,8 | 254,231,28,14 |
| Static/center | 3207 | 0 / 0 (L736) | 172,132,16,8 | 301,231,28,14 |
| Static/center | 3194 | 0 / 0 (L731) | 198,132,19,8 | 346,231,33,14 |
| Static/center | 3210 | 0 / 0 (L737) | 226,132,14,8 | 396,231,24,14 |
| Static/center | 3212 | 0 / 0 (L738) | 253,132,14,8 | 443,231,24,14 |

#### Dialog(387) — 选项页：声音处理（规格化/混响）

模板：`282×237` DLU ≈ `494×415` px；字体 Arial 9pt；style `0x40000440`；资源 RT_DIALOG 387，文件偏移 28833376（1730 字节）；控件 33 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| Button/groupbox | 3385 | Normalizer (Auto Volume Control) / 规格化 (L1248) | 5,3,273,110 | 9,5,478,192 |
| Button/autocheck | 3035 | Enable (for broadcasting) / 在 播放/直播 中使用规格化 (L1234) | 11,14,244,10 | 19,24,427,18 |
| Button/autocheck | 3034 | Enable / 播放时开启规格化 (L1233) | 11,25,244,10 | 19,44,427,18 |
| Button/autocheck | 3036 | Operate only with video playback / 仅将规格化用于视频播放 (L1235) | 18,36,244,10 | 32,63,427,18 |
| Static/etched-h | 3221 |  | 8,49,269,2 | 14,86,471,4 |
| Static/right | 3216 | Method: / 类型： (L1243) | 8,55,89,8 | 14,96,156,14 |
| ComboBox/droplist | 3006 |  | 100,53,173,116 | 175,93,303,203 |
| Button/autocheck | 3047 | Boost audio / 保持放大系数 (L1237) | 100,68,132,10 | 175,119,231,18 |
| Static/right | 3204 | Level (%d%%): / 最大放大级别(%d%%)： (L1241) | 8,81,89,8 | 14,142,156,14 |
| Trackbar/horz | 3353 |  | 98,81,134,10 | 172,142,234,18 |
| Button/push | 3029 | Default / 默认值 (L1230) | 235,78,38,12 | 411,136,66,21 |
| Static/right | 3220 | Release time (%d): / 音量变化级别(%d)： (L1244) | 8,97,89,8 | 14,170,156,14 |
| Trackbar/horz | 3377 |  | 98,97,134,10 | 172,170,234,18 |
| Button/push | 3031 | Default / 默认值 (L1231) | 235,95,38,12 | 411,166,66,21 |
| Button/groupbox | 3392 | Freeverb / 混响 (L1250) | 5,117,273,112 | 9,205,478,196 |
| Button/autocheck | 3072 | Enable / 启用混响 (L1239) | 11,128,65,10 | 19,224,114,18 |
| Static/left | 3396 | * it operates only in 2 channels. / * 仅工作于双声道中 (L1251) | 88,129,184,8 | 154,226,322,14 |
| Static/etched-h | 3222 |  | 8,142,269,2 | 14,248,471,4 |
| Static/right | 3207 | Room size (%g): / 空间尺度(%g)： (L1242) | 8,150,89,8 | 14,262,156,14 |
| Trackbar/horz | 3079 |  | 98,150,134,10 | 172,262,234,18 |
| Button/push | 3027 | Default / 默认值 (L1229) | 235,148,38,12 | 411,259,66,21 |
| Static/right | 3224 | Damping (%g): / 阻尼系数(%g)： (L1245) | 8,166,89,8 | 14,290,156,14 |
| Trackbar/horz | 3098 |  | 98,166,134,10 | 172,290,234,18 |
| Button/push | 3033 | Default / 默认值 (L1232) | 235,164,38,12 | 411,287,66,21 |
| Static/right | 3228 | Wet level (%g): / 湿度(%g)： (L1246) | 8,182,89,8 | 14,318,156,14 |
| Trackbar/horz | 3103 |  | 98,182,134,10 | 172,318,234,18 |
| Button/push | 3042 | Default / 默认值 (L1236) | 235,180,38,12 | 411,315,66,21 |
| Static/right | 3232 | Dry level (%g): / 干度(%g)： (L1247) | 8,198,89,8 | 14,346,156,14 |
| Trackbar/horz | 3108 |  | 98,198,134,10 | 172,346,234,18 |
| Button/push | 3092 | Default / 默认值 (L1240) | 235,196,38,12 | 411,343,66,21 |
| Static/right | 3390 | Width (%g): / 频响宽度(%g)： (L1249) | 8,214,89,8 | 14,374,156,14 |
| Trackbar/horz | 3112 |  | 98,214,134,10 | 172,374,234,18 |
| Button/push | 3056 | Default / 默认值 (L1238) | 235,212,38,12 | 411,371,66,21 |

#### Dialog(383) — 选项页：字幕字体/颜色/位置

模板：`284×244` DLU ≈ `497×427` px；字体 Arial 9pt；style `0x40000440`；资源 RT_DIALOG 383，文件偏移 28823992（4004 字节）；控件 84 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| Button/groupbox | 3286 | Font / 字体 (L1138) | 5,3,273,79 | 9,5,478,138 |
| Static/right | 3306 | Default: / 默认字体： (L1155) | 9,14,45,8 | 16,24,79,14 |
| Button/push | 3021 | Font / 字体 (L1131) | 57,12,124,12 | 100,21,217,21 |
| Static/right | 3307 | Alternative: / 备用字体： (L1156) | 9,28,45,8 | 16,49,79,14 |
| Button/push | 3022 | Font / 字体 (L1132) | 57,27,124,12 | 100,47,217,21 |
| Button/autocheck | 3038 | Restrict fonts to Korean alphabet / 将主要/备用字体视为韩字范围处理 (L1137) | 57,41,121,8 | 100,72,212,14 |
| Static/right | 3288 | Charset: / 字符集： (L1140) | 9,53,45,8 | 16,93,79,14 |
| ComboBox/droplist | 3003 |  | 57,51,124,52 | 100,89,217,91 |
| Static/right | 3308 | Scale X: / 平面宽度： (L1157) | 9,68,45,8 | 16,119,79,14 |
| Edit | 3099 |  | 57,66,36,12 | 100,116,63,21 |
| UpDown | 3263 |  | 85,71,10,14 | 149,124,18,24 |
| Static/right | 3310 | Scale Y: / 平面长度： (L1159) | 96,68,46,8 | 168,119,80,14 |
| Edit | 3105 |  | 145,66,36,12 | 254,116,63,21 |
| UpDown | 3264 |  | 165,60,10,14 | 289,105,18,24 |
| Static/right | 3287 | Size: / 字号： (L1139) | 184,15,51,8 | 322,26,89,14 |
| Edit | 3088 |  | 238,12,36,12 | 416,21,63,21 |
| UpDown | 3262 |  | 269,14,10,14 | 471,24,18,24 |
| Static/right | 3289 | Letter spacing: / 字间距： (L1141) | 184,32,51,8 | 322,56,89,14 |
| Edit | 3077 |  | 238,30,36,12 | 416,52,63,21 |
| UpDown | 3260 |  | 269,30,11,14 | 471,52,19,24 |
| Static/right | 3304 | Line spacing: / 行间距： (L1153) | 184,49,51,8 | 322,86,89,14 |
| Edit | 3095 |  | 238,48,36,12 | 416,84,63,21 |
| UpDown | 3261 |  | 269,47,11,14 | 471,82,19,24 |
| Static/right | 3305 | Ruby position: / <ruby>位置： (L1154) | 184,67,51,8 | 322,117,89,14 |
| Edit | 3096 |  | 238,65,36,12 | 416,114,63,21 |
| UpDown | 3266 |  | 271,62,11,14 | 474,108,19,24 |
| Button/groupbox | 3290 | Colors && Transparency && Outline && Shadow / 颜色&&透明度&&边框&&阴影 (L1142) | 5,84,169,149 | 9,147,296,261 |
| Static/right | 3317 | Color / 颜色 (L1165) | 21,94,51,8 | 37,164,89,14 |
| Static/center | 3312 | Transparency / 透明度 (L1160) | 78,94,49,8 | 136,164,86,14 |
| Static/right | 3315 | Width/Depth / 尺寸/深度 (L1163) | 130,94,40,8 | 228,164,70,14 |
| Static/right | 3291 | Text: / 默认： (L1143) | 9,107,50,8 | 16,187,88,14 |
| Static/(色块) | 3213 |  | 61,105,12,12 | 107,184,21,21 |
| Trackbar/horz | 3045 |  | 76,107,60,10 | 133,187,105,18 |
| Static/right | 3292 | Outline: / 第一边框： (L1144) | 9,121,50,8 | 16,212,88,14 |
| Static/(色块) | 3221 |  | 61,119,12,12 | 107,208,21,21 |
| Trackbar/horz | 3046 |  | 76,121,60,10 | 133,212,105,18 |
| Edit | 3069 |  | 139,118,30,12 | 243,206,52,21 |
| UpDown | 3248 |  | 163,121,10,12 | 285,212,18,21 |
| Static/right | 3296 | 2nd Outline: / 第二边框： (L1146) | 9,135,50,8 | 16,236,88,14 |
| Static/(色块) | 3222 |  | 61,133,12,12 | 107,233,21,21 |
| Trackbar/horz | 3047 |  | 76,135,60,10 | 133,236,105,18 |
| Edit | 3072 |  | 139,133,30,12 | 243,233,52,21 |
| UpDown | 3254 |  | 163,136,10,12 | 285,238,18,21 |
| Static/right | 3293 | Shadow: / 阴影： (L1145) | 9,149,50,9 | 16,261,88,16 |
| Static/(色块) | 3226 |  | 61,147,12,12 | 107,257,21,21 |
| Trackbar/horz | 3048 |  | 76,149,60,10 | 133,261,105,18 |
| Edit | 3082 |  | 139,148,30,12 | 243,259,52,21 |
| UpDown | 3249 |  | 165,153,10,12 | 289,268,18,21 |
| Static/etched-h | -1 |  | 8,163,165,1 | 14,285,289,2 |
| Static/right | 3303 | Outline blur: / 第一边框默认模糊度： (L1152) | 10,168,125,8 | 18,294,219,14 |
| Edit | 3070 |  | 139,166,30,12 | 243,290,52,21 |
| UpDown | 3250 |  | 167,172,10,12 | 292,301,18,21 |
| Static/right | 3309 | Outline gaussian blur: / 第一边框高斯模糊度： (L1158) | 10,182,125,8 | 18,318,219,14 |
| Edit | 3071 |  | 139,181,30,12 | 243,317,52,21 |
| UpDown | 3252 |  | 164,189,10,12 | 287,331,18,21 |
| Static/etched-h | -1 |  | 8,196,165,1 | 14,343,289,2 |
| Button/autocheck | 3036 | Use outline background (uses 1st outline color) / 将第一边框作不透明框处理 (L1135) | 12,199,159,10 | 21,348,278,18 |
| Button/autocheck | 3035 | Scale outline/shadow / 自动调整第一边框/阴影尺寸 (L1134) | 12,209,159,10 | 21,366,278,18 |
| Button/autocheck | 3037 | Scale 2nd outline / 自动调整第二边框尺寸 (L1136) | 12,219,159,10 | 21,383,278,18 |
| Button/groupbox | 3298 | Position && Margin / 位置&&比例 (L1147) | 179,84,99,121 | 313,147,173,212 |
| Static/right | 3318 | Paragraph align: / 段落对齐： (L1166) | 183,94,58,8 | 320,164,102,14 |
| ComboBox/droplist | 3005 |  | 244,92,30,52 | 427,161,52,91 |
| Static/right | 3314 | Horizontal pos(%): / 水平位置(%)： (L1162) | 183,109,58,8 | 320,191,102,14 |
| Edit | 3271 |  | 244,107,30,12 | 427,187,52,21 |
| UpDown | 3257 |  | 272,108,10,12 | 476,189,18,21 |
| Static/right | 3316 | Vertical pos(%): / 垂直位置(%)： (L1164) | 183,125,58,8 | 320,219,102,14 |
| Edit | 3253 |  | 244,123,30,12 | 427,215,52,21 |
| UpDown | 3274 |  | 272,125,10,12 | 476,219,18,21 |
| Static/right | 3299 | Top margin: / 顶部间距： (L1148) | 183,141,58,8 | 320,247,102,14 |
| Edit | 3267 |  | 244,139,30,12 | 427,243,52,21 |
| UpDown | 3255 |  | 272,141,10,12 | 476,247,18,21 |
| Static/right | 3300 | Left margin: / 左侧间距： (L1149) | 183,156,58,8 | 320,273,102,14 |
| Edit | 3247 |  | 244,155,30,12 | 427,271,52,21 |
| UpDown | 3265 |  | 272,157,10,12 | 476,275,18,21 |
| Static/right | 3301 | Right margin: / 右侧间距： (L1150) | 183,173,58,8 | 320,303,102,14 |
| Edit | 3251 |  | 244,171,30,12 | 427,299,52,21 |
| UpDown | 3269 |  | 272,173,10,12 | 476,303,18,21 |
| Static/right | 3302 | Bottom margin: / 底部间距： (L1151) | 183,189,58,8 | 320,331,102,14 |
| Edit | 3272 |  | 244,187,30,12 | 427,327,52,21 |
| UpDown | 3256 |  | 272,189,10,12 | 476,331,18,21 |
| Button/groupbox | 3313 | Opaque box / 边框不透明度 (L1161) | 179,207,99,26 | 313,362,173,46 |
| Button/auto3state | 3034 | Enable / 使用 (L1133) | 184,217,35,10 | 322,380,61,18 |
| Static/(色块) | 3214 |  | 222,216,12,12 | 388,378,21,21 |
| Trackbar/horz | 3044 |  | 237,218,37,10 | 415,382,65,18 |

#### Dialog(364) — 选项页：时间跨度（步进秒数）

模板：`284×157` DLU ≈ `497×275` px；字体 Arial 9pt；style `0x40000440`；资源 RT_DIALOG 364，文件偏移 28780216（1352 字节）；控件 25 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| Button/groupbox | 3311 | Time jump settings / 定位时间设置 (L760) | 5,3,273,140 | 9,5,478,245 |
| Static/left | 3191 | Here you can set time jump intervals and frame s / 可设置播放时以时间为单位的移动跨度。 (L755) | 13,14,262,11 | 23,24,458,19 |
| Static/right | 3381 | Left/Right Arrow Keys: / 左/右 方向键： (L761) | 9,32,157,8 | 16,56,275,14 |
| Edit | 3267 |  | 170,30,36,12 | 298,52,63,21 |
| UpDown | 3255 |  | 203,30,10,12 | 355,52,18,21 |
| Static/left | 3194 | Sec. / 秒 (L756) | 209,32,17,8 | 366,56,30,14 |
| Static/right | 3382 | Ctrl + Left/Right Arrow Keys: / Ctrl + 左/右 方向键： (L762) | 9,47,157,8 | 16,82,275,14 |
| Edit | 3270 |  | 170,46,36,12 | 298,80,63,21 |
| UpDown | 3256 |  | 203,46,10,12 | 355,80,18,21 |
| Static/left | 3197 | Sec. / 秒 (L757) | 209,47,17,8 | 366,82,30,14 |
| Static/right | 3383 | Shift + Left/Right Arrow Keys: / Shift + 左/右 方向键： (L763) | 9,64,157,8 | 16,112,275,14 |
| Edit | 3271 |  | 170,62,36,12 | 298,108,63,21 |
| UpDown | 3257 |  | 203,62,10,12 | 355,108,18,21 |
| Static/left | 3402 | Sec. / 秒 (L765) | 209,64,17,8 | 366,112,30,14 |
| Static/right | 3384 | Ctrl + Alt + Left/Right Arrow Keys: / Ctrl + Alt + 左/右 方向键： (L764) | 9,79,157,8 | 16,138,275,14 |
| Edit | 3272 |  | 170,78,36,12 | 298,136,63,21 |
| UpDown | 3258 |  | 203,78,10,12 | 355,136,18,21 |
| Static/left | 3202 | Sec. / 秒 (L758) | 209,79,17,8 | 366,138,30,14 |
| Button/push | 3021 | Default / 默认值 (L750) | 229,30,43,12 | 401,52,75,21 |
| Button/push | 3024 | Default / 默认值 (L751) | 229,46,43,12 | 401,80,75,21 |
| Button/push | 3027 | Default / 默认值 (L752) | 229,62,43,12 | 401,108,75,21 |
| Button/push | 3029 | Default / 默认值 (L753) | 229,78,43,12 | 401,136,75,21 |
| Static/etched-h | -1 |  | 8,95,268,2 | 14,166,469,4 |
| Button/autocheck | 3034 | Jump to keyframe / 如存在关键帧数据则以关键帧为移动单位 (L754) | 19,102,256,10 | 33,178,448,18 |
| Static/left | 3204 | With using keyframe search, time jumping can be  / 按时间搜索不正确时可用此方式进行快速预览。 (L759) | 30,114,242,23 | 52,200,424,40 |

#### Dialog(337) — 连续截图（推断对应 ID_VIDEO_ADV_CAPTURE 10222）

模板：`250×281` DLU ≈ `438×492` px；字体 Arial 9pt；style `0x80000040`；资源 RT_DIALOG 337，文件偏移 28765440（2004 字节）；控件 45 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| Button/groupbox | 3285 | Storage / 保存设置 (L351) | 11,20,229,60 | 19,35,401,105 |
| Edit | 3069 |  | 16,31,199,12 | 28,54,348,21 |
| Button/push | 3021 | ... / ... (L341) | 218,31,17,12 | 382,54,30,21 |
| Static/right | 3297 | Filename prefix: / 文件前缀名： (L362) | 17,49,68,8 | 30,86,119,14 |
| Edit | 3279 |  | 88,47,147,12 | 154,82,257,21 |
| Static/right | 3299 | Filename suffix: / 命名文件方式： (L364) | 17,64,68,8 | 30,112,119,14 |
| ComboBox/droplist | 3004 |  | 88,63,147,100 | 154,110,257,175 |
| Button/groupbox | 3286 | Image Type / 格式设置 (L352) | 11,83,112,51 | 19,145,196,89 |
| Static/right | 3287 | Format: / 格式： (L353) | 15,97,36,8 | 26,170,63,14 |
| ComboBox/droplist | 3003 |  | 54,95,62,100 | 94,166,108,175 |
| Static/right | 3288 | Quality: / 质量： (L354) | 15,115,36,8 | 26,201,63,14 |
| Edit | 3082 |  | 54,113,43,12 | 94,198,75,21 |
| UpDown | 3248 |  | 84,120,10,14 | 147,210,18,24 |
| Static/left | 3293 | % / % (L358) | 102,115,15,8 | 178,201,26,14 |
| Button/groupbox | 3292 | Image Size / 尺寸设置 (L357) | 128,83,112,51 | 224,145,196,89 |
| Button/autoradio | 3209 | Original / 截取为原始尺寸 (L349) | 132,93,102,10 | 231,163,178,18 |
| Button/autoradio | 3154 | Custom / 截取为自定义尺寸 (L347) | 132,105,102,10 | 231,184,178,18 |
| Edit | 3254 |  | 137,118,37,12 | 240,206,65,21 |
| UpDown | 3250 |  | 158,125,10,14 | 276,219,18,24 |
| Static/left | 3294 | × / × (L359) | 177,120,8,8 | 310,210,14,14 |
| Edit | 3147 |  | 184,118,37,12 | 322,206,65,21 |
| UpDown | 3258 |  | 204,126,10,14 | 357,220,18,24 |
| Button/push | 3358 | G / G (L366) | 224,118,12,12 | 392,206,21,21 |
| Button/groupbox | 3300 | Numbers/Frames to Capture / 截图设置 (L365) | 11,136,112,58 | 19,238,196,102 |
| Static/right | 3290 | Number(s): / 采集数量： (L356) | 16,148,62,8 | 28,259,108,14 |
| Edit | 3088 |  | 81,146,37,12 | 142,256,65,21 |
| UpDown | 3249 |  | 104,139,11,14 | 182,243,19,24 |
| Button/autoradio | 3210 | Every # ms / 按时间 每毫秒 (L350) | 20,164,56,10 | 35,287,98,18 |
| Button/autoradio | 3155 | Every # frame / 按帧间隔 每帧 (L348) | 20,179,56,10 | 35,313,98,18 |
| Edit | 3095 |  | 81,162,37,12 | 142,284,65,21 |
| UpDown | 3252 |  | 102,166,11,14 | 178,290,19,24 |
| Edit | 3096 |  | 81,178,37,12 | 142,312,65,21 |
| UpDown | 3253 |  | 102,180,11,14 | 178,315,19,24 |
| Button/groupbox | 3289 | When Capturing / 其他设置 (L355) | 128,136,112,58 | 224,238,196,102 |
| Button/autocheck | 3034 | Include messages, subtitles / 同时截取字幕(如果有) (L344) | 134,147,100,9 | 234,257,175,16 |
| Button/autocheck | 3035 | Correct aspect ratio / 按调整后的比例保存 (L345) | 134,160,100,9 | 234,280,175,16 |
| Button/groupbox | 3295 | Activity / 信息 (L360) | 11,196,229,61 | 19,343,401,107 |
| Static/right | 3296 | Filename: / 收尾文件名： (L361) | 18,208,56,8 | 32,364,98,14 |
| Edit | 3270 |  | 78,208,157,12 | 136,364,275,21 |
| Static/right | 3298 | Progress: / 文件数： (L363) | 18,222,56,8 | 32,388,98,14 |
| Static/left | 3381 |  | 78,222,155,8 | 136,388,271,14 |
| Static/etched-h | -1 |  | 14,234,224,1 | 24,410,392,2 |
| Button/push | 3024 | Open Storage Folder / 打开存档文件夹 (L342) | 15,238,79,14 | 26,416,138,24 |
| Button/push | 3027 | Capture Current Frame / 截取当前画面 (L343) | 96,238,79,14 | 168,416,138,24 |
| Button/push | 3038 | Start / 开始 (L346) | 177,238,58,14 | 310,416,102,24 |

#### Dialog(413) — 录制视频（推断对应 ID_SAVE_MOVIE 10370）

模板：`274×323` DLU ≈ `480×565` px；字体 Arial 9pt；style `0x80000840`；资源 RT_DIALOG 413，文件偏移 28808192（2200 字节）；控件 53 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| Button/groupbox | 3286 | Storage / 保存设置 (L1531) | 12,20,252,76 | 21,35,441,133 |
| Edit | 3069 |  | 17,30,225,12 | 30,52,394,21 |
| Button/push | 3021 | ... / ... (L1519) | 245,30,15,11 | 429,52,26,19 |
| Static/right | 3294 | Filename prefix: / 文件前缀名： (L1538) | 18,48,80,8 | 32,84,140,14 |
| Edit | 3095 |  | 102,46,157,12 | 178,80,275,21 |
| Static/etched-h | -1 |  | 15,62,247,1 | 26,108,432,2 |
| Button/autocheck | 3040 | Split files / 总是使用 (L1523) | 19,67,65,10 | 33,117,114,18 |
| Button/autoradio | 3406 | Every # minutes / 按时间单位中断 (L1547) | 90,67,79,10 | 158,117,138,18 |
| Button/autoradio | 3036 | Every # mb / 按字节单位中断 (L1521) | 174,66,84,10 | 304,116,147,18 |
| Edit | 3088 |  | 102,79,35,12 | 178,138,61,21 |
| UpDown | 3248 |  | 132,82,11,12 | 231,144,19,21 |
| Static/left | 3287 | Min. / 分 (L1532) | 141,81,31,8 | 247,142,54,14 |
| Edit | 3099 |  | 185,79,40,12 | 324,138,70,21 |
| UpDown | 3249 |  | 219,83,11,12 | 383,145,19,21 |
| Static/left | 3288 | MB / 兆 (L1533) | 230,81,29,8 | 402,142,51,14 |
| Button/groupbox | 3394 | Settings / 输出设置 (L1543) | 12,100,252,131 | 21,175,441,229 |
| Static/right | 3197 | File Format: / 文件格式： (L1526) | 16,111,57,8 | 28,194,100,14 |
| ComboBox/droplist | 3003 |  | 77,109,112,140 | 135,191,196,245 |
| Static/right | 3398 | Speed: / 速度： (L1544) | 192,111,30,8 | 336,194,52,14 |
| Edit | 3403 |  | 227,109,32,12 | 397,191,56,21 |
| UpDown | 3250 |  | 252,114,10,12 | 441,200,18,21 |
| Static/etched-h | -1 |  | 15,125,247,2 | 26,219,432,4 |
| Static/right | 3402 | Video Codec: / 视频编码： (L1545) | 16,131,57,8 | 28,229,100,14 |
| ComboBox/droplist | 3005 |  | 77,129,165,138 | 135,226,289,242 |
| Button/push | 3330 | ... / ... (L1539) | 245,129,15,11 | 429,226,26,19 |
| Static/right | 3404 | Bitrate: / 图像质量： (L1546) | 16,146,57,8 | 28,256,100,14 |
| Edit | 3105 |  | 77,145,48,12 | 135,254,84,21 |
| UpDown | 3264 |  | 111,148,10,14 | 194,259,18,24 |
| Static/left | 3384 | kbps / kbps (L1542) | 129,147,26,8 | 226,257,46,14 |
| Button/autocheck | 3034 | Use HQ compression / 开启高质量压缩 (L1520) | 159,146,99,10 | 278,256,173,18 |
| Static/right | 3202 | Size: / 图像尺寸： (L1527) | 16,163,57,8 | 28,285,100,14 |
| ComboBox/dropdown | 3011 |  | 77,161,75,138 | 135,282,131,242 |
| ComboBox/droplist | 3051 |  | 159,161,100,129 | 278,282,175,226 |
| Static/right | 3205 | FPS: / 帧率： (L1528) | 16,178,57,8 | 28,312,100,14 |
| ComboBox/dropdown | 3006 |  | 77,176,75,122 | 135,308,131,214 |
| Button/autocheck | 3047 | Use buffering / 采用多线程压缩视频 (L1525) | 159,177,99,10 | 278,310,173,18 |
| Static/etched-h | -1 |  | 15,191,247,2 | 26,334,432,4 |
| Static/right | 3208 | Audio Codec: / 声音编码： (L1529) | 16,198,57,8 | 28,346,100,14 |
| ComboBox/droplist | 3008 |  | 77,196,109,129 | 135,343,191,226 |
| ComboBox/droplist | 3010 |  | 190,196,69,122 | 332,343,121,214 |
| Static/right | 3216 | Sample Rate: / 采样率： (L1530) | 16,214,57,8 | 28,374,100,14 |
| ComboBox/droplist | 3014 |  | 77,212,109,129 | 135,371,191,226 |
| ComboBox/droplist | 3017 |  | 190,212,69,122 | 332,371,121,214 |
| Button/groupbox | 3289 | Activity / 信息 (L1534) | 12,235,252,57 | 21,411,441,100 |
| Static/right | 3290 | Filename: / 文件名： (L1535) | 16,246,57,8 | 28,430,100,14 |
| Edit | 3082 |  | 77,246,181,12 | 135,430,317,21 |
| Static/right | 3291 | Rec. time: / 存储时间： (L1536) | 16,258,57,8 | 28,452,100,14 |
| Static/left | 3381 | 00:00:00 / 00:00:00 (L1540) | 77,258,52,8 | 135,452,91,14 |
| Static/right | 3293 | File size: / 容量： (L1537) | 132,258,58,8 | 231,452,102,14 |
| Static/left | 3383 | 0 MB / 0 MB (L1541) | 194,258,64,8 | 340,452,112,14 |
| Static/etched-h | -1 |  | 15,269,247,2 | 26,471,432,4 |
| Button/push | 3038 | Open Storage Folder / 打开存档文件夹 (L1522) | 17,274,121,13 | 30,480,212,23 |
| Button/push | 3042 | Start / 开始 (L1524) | 142,274,118,13 | 248,480,206,23 |

#### Dialog(424) — 播放/系统信息（推断对应 ID_PLAYINFO_WINDOW 10195）

模板：`287×309` DLU ≈ `502×541` px；字体 Arial 9pt；style `0x86000040`；资源 RT_DIALOG 424，文件偏移 28846128（472 字节）；控件 8 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| Button/push | 3035 | Co&py to the clipboard / 复制到剪贴板(&P) (L1646) | 55,254,95,14 | 96,444,166,24 |
| Button/push | 3038 | Playback Info / 播放信息 (L1648) | 8,14,50,14 | 14,24,88,24 |
| Button/push | 3025 | File Resource / 媒体内嵌资源 (L1643) | 61,14,79,14 | 107,24,138,24 |
| Button/push | 3024 | File Info / 文件信息 (L1642) | 143,14,42,14 | 250,24,74,24 |
| Tab | 3380 |  | 7,37,208,171 | 12,65,364,299 |
| Button/push | 3036 | &Save as... / 保存为其他名称(&S)... (L1647) | 54,270,95,14 | 94,472,166,24 |
| Button/push | 3026 | System Info / 系统信息 (L1644) | 188,14,47,14 | 329,24,82,24 |
| Button/push | 3027 | Online Info / 在线信息 (L1645) | 239,14,47,14 | 418,24,82,24 |

#### Dialog(403) — 专辑编辑（推断对应 10002/10003）

模板：`444×221` DLU ≈ `777×387` px；字体 Arial 9pt；style `0x86000040`；资源 RT_DIALOG 403，文件偏移 28794768（476 字节）；控件 10 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| ListView | 3242 |  | 15,24,415,156 | 26,42,726,273 |
| Button/push | 3021 | Add file(s)... / 添加文件 (L1426) | 15,185,41,11 | 26,324,72,19 |
| Button/push | 3024 | Add folder... / 添加文件夹 (L1427) | 58,185,41,11 | 102,324,72,19 |
| Button/push | 3025 | Add URL... / 添加链接 (L1428) | 101,185,41,11 | 177,324,72,19 |
| Button/push | 3027 | Delete / 移除 (L1429) | 145,185,41,11 | 254,324,72,19 |
| Button/push | 3029 | Edit... / 编辑 (L1430) | 188,185,41,11 | 329,324,72,19 |
| Button/push | 3031 | Up / 上移 (L1431) | 231,185,41,11 | 404,324,72,19 |
| Button/push | 3033 | Down / 下移 (L1432) | 273,185,41,11 | 478,324,72,19 |
| Button/push | 3035 | Add to playlist / 添加到播放列表 (L1433) | 316,185,71,11 | 553,324,124,19 |
| Button/push | 3039 | Play / 立即播放 (L1434) | 389,185,41,11 | 681,324,72,19 |

#### Dialog(324) — 选项（参数选项）框架（ID_OPTION_WINDOW 10010）

模板：`429×301` DLU ≈ `751×527` px；字体 Arial 9pt；style `0x80000040`；资源 RT_DIALOG 324，文件偏移 28752744（396 字节）；控件 7 个。

| 类型 | id | 文本（en / zh，语言文件行号） | DLU x,y,w,h | px x,y,w,h |
| --- | --- | --- | --- | --- |
| ComboBox/droplist | 3005 |  | 9,21,117,145 | 16,37,205,254 |
| Edit | 3069 |  | 9,36,117,12 | 16,63,205,21 |
| TreeView | 3315 |  | 9,51,117,226 | 16,89,205,396 |
| Tab | 3380 |  | 131,21,289,257 | 229,37,506,450 |
| Button/push | 3035 | &Reset / 初始化(&I) (L200) | 142,281,46,14 | 248,492,80,24 |
| Button/push | 3042 | &Export Settings... / 导出当前配置(&S)... (L201) | 191,281,102,14 | 334,492,178,24 |
| Button/push | 3043 | &Apply / 应用(&A) (L202) | 295,281,62,14 | 516,492,108,24 |


## 7. 未知项清单（需 Windows 实机确认）

| # | 未知项 | 静态证据到哪一步 | 需要什么来确认 |
| --- | --- | --- | --- |
| U1 | 对话框运行时字体：模板是 Arial 9pt，语言文件 `DefaultMessageFont` 为 宋体（中文）/ Segoe UI（英文），是否替换 | 模板字段；`Chinese(Simplified).ini:4`、`English.ini:3` | 中文界面下打开 Dialog 407/324，用 Spy++ 或 WinSpy 看 WM_GETFONT 的字体名和字号；或截图量一个 12 DLU 编辑框的像素高度（21 px = Arial；18 px = 宋体） |
| U2 | 默认右键菜单是完整版（RT_MENU 101）还是 SimpleMenu.xml；“菜单输出方式/弹出菜单类型”（4742/4743）的默认值 | 两套菜单都存在 | 新装 PotPlayer 默认配置下右键截图；查看 选项 → 皮肤（Dialog 376）的菜单相关设置 |
| U3 | 底部控制栏在窗口/全屏模式下的显示与自动隐藏规则；有无隐藏控制栏的快捷键 | 静态菜单与加速键表都没有对应命令；仅有 10369 | 实机操作：全屏、窗口、开关“视频下自动隐藏”时截图；查看 选项 → 快捷键 列表 |
| U4 | 弹出式对话框的自绘标题栏高度、边框、标题文字，以及确定/取消/关闭按钮是否运行时加入、尺寸和位置 | 模板无 WS_CAPTION、几乎无 IDOK/IDCANCEL，内容从 y≈20 DLU 开始 | 打开 407、310、324、337 截图（100% 缩放），量标题栏高度、按钮尺寸与右下边距 |
| U5 | 右键菜单的绘制样式（自绘/系统）：菜单项高度、字体、分隔线、勾选与单选圆点的外观、颜色 | 导入了 `SetMenuItemBitmaps`、`GetMenuCheckMarkDimensions`、`SetMenuInfo`，可能自绘 | 100% DPI 下右键菜单截图，量行高与左侧勾选列宽 |
| U6 | 均衡器预设名称和数量（`声音 → 预置` 20008 为运行时占位） | 菜单 106 只有 添加/保存/移除/初始化；符号表中 20008 另被注册为 `CMD_CLIENT_CHAT_MODE`，说明该 ID 在菜单里只是占位 | 实机展开 声音 → 预置 截图 |
| U7 | 部分菜单项的勾选形式：10069 无序播放、10419 记忆播放位置、10199–10201 等处理函数通过间接跳转（`jmp rax/r9/rcx`），静态无法判定是复选还是仅启用/禁用 | 处理函数地址 `0x1803ca340`、`0x1803ca170`、`0x1803ca510` | 实机展开对应菜单看是否有勾 |
| U8 | Open URL Dialog(310) 运行时布局：模板中 ListView、Tab、按钮、下拉框互相重叠 | 模板坐标（§6.2） | 打开链接对话框截图 |
| U9 | 播放速度增减步长（X/C 每次变化量）与速度上下限 | 只有 OSD 文本 `1100=Playback speed : %.3gx`（L3481）和选项 `3292=Speed (%d):` | 实机按 C/X 观察 OSD；查看 选项 → 播放 的速度设置 |
| U10 | 左右方向键默认跳转秒数（推断为 5 秒） | 符号名 `ID_PLAYBACK_5SEC_BACK/FORW`；Dialog 364 模板不含默认值；`0x1803ca0f0` 从 `[this+0x182c]` 读取秒数 | 打开 选项 → 时间跨度 页，读默认值；或看菜单“步退%s”实际文字 |
| U11 | 对话框与命令的对应（407↔10325、310↔10159、463↔10865/10878、461↔10851、337↔10222、413↔10370、424↔10195、403↔10002/10003、412↔10404）以及“字幕浏览器 10228”“画面调节”对应的实际窗口 | 只由文字内容推断；资源中无命令→对话框映射 | 逐个执行菜单命令截图核对 |
| U12 | 高 DPI（125%/150%）下对话框与控制面板皮肤的缩放方式（DLU 随字体缩放；皮肤像素是否按 DPI 放大） | 无静态证据 | 125%、150% 缩放下截图对比 |
| U13 | 选项页在框架 Dialog(324) 中的实际放置位置和树节点名称/顺序（哪个树节点打开哪个页模板） | 页模板尺寸与 Tab 客户区吻合；树内容运行时生成 | 打开选项窗口逐页截图 |
| U14 | 主窗口与播放列表窗口加速键作用范围：PL 表 118 中 F6→10446、Ctrl+Up/Down→27020/27021 是否只在列表获得焦点时生效；PgUp/PgDn 实际是 10123/10124 还是 10067/10068 的行为 | 两张加速键表与菜单文字不一致 | 实机分别在视频区和列表区按键观察 |

未知项共 **14** 条。
