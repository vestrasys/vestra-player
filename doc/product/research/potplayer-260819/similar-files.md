# PotPlayer「打开文件时加入相似文件」规则（静态逆向）

日期：2026-10-21。对象：PotPlayer 260819 x64，`PotPlayer64.dll`（文件版本 1.7.23122.0）。用途：给 Vestra playlist 插件的 IA-PL-01 / A13 做功能兼容，规格写在 [interaction-alignment.md](../../interaction-alignment.md) 的 IA-PL-01「同类文件规则」小节。

## 0. 方法与边界

- 安装包：按 `metadata.json` 的 URL 下载，sha256 `4b9833ad…f9cfe` 核对一致，`7zz x` 解包。全程**没有运行**任何 Windows 程序，只做静态分析。
- 工具：macOS 自带 llvm `objdump -d`（全量反汇编，用于搜索立即数和字段偏移）；Ghidra 12.1.4 headless（通过 brew 安装），导入时关闭自动分析（`-noanalysis`），只对下面列出的函数做反编译。导入表、字符串由自写的只读脚本按 PE 结构解析。
- 只分析与本功能有关的函数。仓库里不存放二进制和大段反汇编，下面只记地址、常量和归纳出的逻辑。地址都是这个 DLL 的虚拟地址（镜像基址 0x180000000），换版本后不再适用。
- 可信度：**确证** = 从指令或反编译直接读出，下文附地址；**推断** = 从调用关系或命名推出，没有逐条验证。

## 1. 选项与默认值

| 项 | 键 / 资源 | 默认 | 取值 | 可信度 |
| --- | --- | --- | --- | --- |
| 打开文件时（相似文件策略） | `OpenWithSameName`（int，配置偏移 0x534）；基本设置 Dialog(354) 3288「When files opened: / 相似文件策略：」；菜单 `101_0_15_5_0..2`，CmdID 10277/10278/10279 | 1 | 0 = 仅打开选定的文件，1 = 同时打开相似文件，2 = 同时打开全部文件 | 确证：菜单处理函数 0x1803b8400 把 `CmdID − 0x2825` 写入 `main+0x6ac`（= 配置基址 main+0x178 再加 0x534）；0x1803cab20 的菜单勾选判断用同一个值；OSD 文字为字符串 1704–1706 |
| 打开新文件时追加而非清空 | `AppendFilePL`；Dialog(373) 3119 | 0（清空） | 0/1 | 默认值确证（见 C §2.4）；对清空的影响见 §4，属推断 |
| 监视正在播放的文件夹 | `AutoAddPL`（配置偏移 0x1472）；Dialog(373) 3115「Auto add similar files in play folder」 | 0 | 0/1 | 确证：定时器 0x1803c23aa 检查 `main+0x15ea`（= 0x178 + 0x1472）以及模式 ≠ 0，再用 `FindFirstChangeNotificationW` 监视文件夹，有变化时用同一套规则重新收集，只追加列表里还没有的文件 |
| 过滤重复文件 | `NoSameFileAddPL`；Dialog(373) 3124 | 0 | 0/1 | 只有默认值；这里没有分析 |

## 2. 调用位置

| 位置 | 作用 | 可信度 |
| --- | --- | --- |
| 0x18046e705–0x18046e7a1（打开文件流程） | 同时满足下列条件时调用收集函数 `0x180455d90(list, mode >= 2)`：调用参数允许（栈参数 +0x370）、模式 ≠ 0、打开的不是 `PotPlayer.m3u`、`0x180853230(path)` 返回假。如果列表由 1 项变成多项：先记下原来那一项（即打开的文件）作为起始项（`job+0x260`），再调用 `0x1804d5170(list, 0)` 排序。`AppendFilePL == 0` 时还会置标志 `job+0x294 = 1` | 调用关系、起始项、排序：确证。各条件的含义：推断（`0x180853230` 推测是判断网络路径或压缩包内路径） |
| 0x180465c84 | 另一个打开入口（推断是拖放或外部传入），做法相同：先收集，再 `0x1804d5170(list, 0)` 排序 | 确证 |
| 0x18058acfb、0x18058b088 | 位于对外接口区，「全部」参数由调用方计数决定 | 只记录了位置 |
| 0x1803c24cb | §1 中的「监视文件夹」定时器 | 确证 |

## 3. 收集算法

`0x180455d90(this, list, all)`：调用 `0x180453f70(this, list, ".*", minLen = all ? 0 : 2, 0)`。

`0x180453f70`（下面称 P 为打开的文件）：

1. **只处理单个文件**：`list` 必须正好 1 项，并且路径不能以 `-` 开头（命令行开关），否则什么也不做。所以多选打开时不扩展。确证：0x180453fd6 至 0x180454004。
2. 拆分 P：取文件夹 `dir`；`ext` 取最后一个扩展名，带点，转小写（`PathFindExtensionW` + `MakeLower`）；`title` 取去掉扩展名的文件名，保留原大小写（`PathRemoveExtensionW` + `PathStripPathW`）。确证：0x1800adb40、0x1803fdc40、0x1800d2dd0、0x180103980。
3. **相似模式的前置条件**（`minLen = 2`）：`title` 中必须至少有一个空格、`-`、`_` 或 ASCII 数字 `0-9`（0x18045445a 处位掩码 `0x80000000_03FF2001`，字符减 0x20 后查表），并且 `title` 长度至少 2。不满足时**一个相似文件也不加**。全部模式（`minLen = 0`）跳过这一步。确证。
4. 类型表：从扩展名注册表（`main+0x4618`）取两组扩展名，各自拼成 `*.avi;*.wmv;…;*.*` 的形式再转小写。第 0 组是「Video files」(0x617)，第 1 组是「Audio files」(0x618)（0x180479d10、0x180478c00、0x180478cf0）。默认扩展名由 0x1804785c0 写入（字符串 0x181384410 / 0x1813845c0 / 0x1813846b0）：
   - 视频：`avi wmv wmp wm asf mpg mpeg mpe m1v m2v mpv2 mp2v ts tp tpr trp vob ifo ogm ogv mp4 m4v m4p m4b 3gp 3gpp 3g2 3gp2 mkv rm ram rmvb rpm flv mov qt nsv dpg m2ts m2t mts dvr-ms k3g skm evo nsr amv divx webm wtv f4v mxf`
   - 音频：`wav wma mpa mp2 m1a m2a mp3 ogg m4a aac mka ra flac ape mpc mod ac3 eac3 dts dtshd wv tak cda dsf tta aiff aif opus amr`
   - 列表文件（第 2 组，这里不使用）：`asx m3u m3u8 pls wvx wax wmx cue mpls mpl dpl xspf mpd`

   用户在设置里改过关联扩展名时，这两组的内容可能不同（推断）。
5. 枚举 `dir\*.*`（`0x1808633c0` → `FindFirstFile`/`FindNextFileW`，0x1804559aa）。**不递归**，跳过目录（属性含 0x10）。**不跳过隐藏文件**。没有数量上限。对每个文件 F：
   1. 取 F 的 `ext'`（小写）和 `stem'`（去掉扩展名，保留原大小写）。
   2. 文件大小为 0 且 `ext'` 在另一张注册表（`main+0x4620`）里时跳过（0x1804552ea，推断：用来排除未下载完的空文件）。
   3. `ext'` 为 `.zip` 或 `.rar` 时跳过（0x18045511c）。
   4. `stem' == title` 并且 `ext' == ext`（`wcscmp`，区分大小写）时跳过，这就是 P 本身（0x18045540d–0x180455470）。
   5. **同类判定**（`wcsstr` 子串查找，0x1804556b5–0x180455869）：
      - P 的 `ext` 和 F 的 `ext'` 都出现在音频表里：属于音频组，转第 6 步，`audio = 1`；
      - 否则两者都出现在视频表里，并且 `ext'` **不在**音频表里：属于视频组，转第 6 步，`audio = 0`；
      - 都不满足：跳过。

      结果是视频只配视频（例如 mkv 配 mp4、avi），音频只配音频（mp3 配 flac）。
   6. 全部模式：直接加入。相似模式：`similar(lower(title)+".tmp", lower(stem')+".tmp", audio)` 为真时才加入（0x18045577b、0x1804558d0）。
   7. 加入方式：把 `dir + 文件名` 追加到列表末尾，顺序就是 `FindNextFile` 返回的顺序（0x180093d90）。
6. 回到调用方后排序（§4）。

## 4. 排序、起始项和列表

- 排序：`0x1804d5170(list, 0)` 把列表放进数组后用 `qsort`，比较函数是 0x1804d5850 → 0x1804d5880。普通文件项走 `0x1804bc000(path1, path2)`，它调用运行时从 `Shlwapi.dll` 取到的 **`StrCmpLogicalW`**（0x1804ba821 用 `GetProcAddress` 取得，存在 0x181a66ad0）。取不到时退回内置的自然排序：数字段按数值比较，其余字符逐个做不区分大小写的比较。比较的是**完整路径**，同一文件夹下等于比较文件名。确证。
- 起始项：排序前记下的原列表唯一一项，也就是打开的文件 P（0x18046e790）。所以列表里排在 P 前面的集数也会加入，播放从 P 开始。确证。
- 清空：`AppendFilePL = 0`（默认）时打开新文件会先清空列表，依据是选项文字和默认值（推断，没有逐条跟到清空的代码）。0x18046e779 的 `job+0x294` 标志只在「加入了相似文件且 `AppendFilePL = 0`」时设置，具体作用没有确定。

## 5. 相似判定 `similar(a, b, audio)`（0x180458b80）

输入：`a` = 打开文件的 `lower(title) + ".tmp"`，`b` = 候选文件的 `lower(stem') + ".tmp"`。代码先把扩展名改成 `.tmp`（`PathRenameExtensionW`），再整体转小写。所以各步的长度都把 `.tmp` 的 4 个字符算在内（删去 `T1` 后剩下 `tmp` 3 个字符），实现时不能省略。小写转换用 `_wcslwr_s`（0x1811914a0），C 区域设置下只转换 A–Z（推断：程序若设置过区域，其它字母也可能被转换）。下面的「长度」都是 UTF-16 码元个数，除法都是整数除法。

常量：

- `T1` = 空格 `_ - . ( ) { } [ ] ' "`，共 12 个字符（0x181a20ce0）；
- `D` = `0-9`（0x181a20cc8）；
- `isdigit` 只认 ASCII `0-9`（0x1811a2db0，ctype 的 `_DIGIT` 位）；调用方先把 ≥ 0x81 的字符换成 0，所以全角数字和中文数字都**不算**数字。

辅助函数：

- `digitDiff(x, y, n)`（0x1804570d0）：看 x、y 的前 n 个字符，只要有一个不是数字就返回 100；否则返回其中不相同的位数。
- `loosePrefix(x, y)`（0x1804586a0）：先删去 x、y 中所有 `T1` 字符。设 `mx` 为较长者的长度，`n = 较短者长度 × 2 / 3`。对 `i < n` 逐位比较：相同则 `match + 1`；不同时，如果两个字符都 < 0x80 且至少一个是数字，就停止比较；其它不同（字母或非 ASCII）跳过，继续比较。最后返回 `mx / 2 <= match`。
- `maskEq(x, y, n, prefix)`（0x1804589d0，通配符为 `*`）：`prefix` 为假时，长度不同直接返回假。对 `i < min(len)` 逐位比较：
  - 字符相同：`prefix` 为真且 `i >= n` 时返回真，否则继续；
  - 字符不同：只要有一边是 `*` 就返回假，否则返回 `i >= n`。

  比完 `min(len)` 个字符都没有提前返回时，结果为真。
- `mask(s)`（0x18046be10，两次调用的参数都是 `insert=1`）：`T1` 字符换成 `?`，数字换成 `*`，长度不变。

主流程（每一步满足条件就判为相似，否则进入下一步）：

```
S1  if min(len a/2, len b/2) >= 5:
        if loosePrefix(a, b): 相似
        if digitDiff(a, b, min(len a/3, len b/3)) <= 2: 相似
S2  a2, b2 = 删去 T1 后的 a, b
    if min(len a2/2, len b2/2) >= 5 and digitDiff(a2, b2, min(len a2/3, len b2/3)) <= 2: 相似
S3  a3, b3 = 再删去数字后的 a2, b2
    if a3 == b3: 相似                                   （区分大小写，但两边都已小写）
    if min(len a3/2, len b3/2) >= 5 and maskEq(a3, b3, min(len a3/3, len b3/3, 5), prefix=true): 相似
S4  am, bm = mask(a), mask(b)                            （用原始 a、b）
    if audio and (maskEq(am, bm, 2, prefix=false) or maskEq(am, bm, 3, prefix=true)): 相似
    if min(len am/2, len bm/2) < 5: 不相似
    返回 maskEq(am, bm, min(len am/3, len bm/3, 5), prefix=true)
```

全部确证：分支、常量、参数顺序都已对照 0x180458b80 的反汇编，在 0x180458bf8、0x180458c36、0x180458ce9、0x180459389–0x1804593c0 等处核对过。

### 5.1 各步的直观含义

| 步骤 | 能匹配上的命名 |
| --- | --- |
| S1 `loosePrefix` | 去掉标点后，在较短者前三分之二的范围内，相同字符数达到较长者长度的一半；遇到「数字对不上」就停止计数。典型：`Show.S01E01` 与 `Show.S01E02`、`Lecture 1` 与 `Lecture 12` |
| S1/S2 `digitDiff` | 两个名字都以较长的数字开头，并且开头部分只差 ≤ 2 位，例如日期 `20230501_x` 与 `20230502_y` |
| S3 相等 | 去掉标点和全部数字后完全相同：`A01`/`A02`、`第01集`/`第10集`、`S01E10`/`S02E01` |
| S3 前缀 | 去掉标点和数字后，两者都至少 10 个字符，并且前 `min(len/3, 5)` 个字符相同。例如同一字幕组前缀 `[字幕组] 进击的巨人` 与 `[字幕组] 鬼灭之刃` |
| S4 掩码 | 标点和数字位置也要对齐：前 `min(len/3, 5)` 个字符相同（数字位必须对数字位），例如 `Show.S01E01` 与 `Shower.Cam` |
| S4 音频专用 | 只看前 3 个掩码字符，或者长度相同且前 2 个掩码字符相同。`01 - Intro` 与 `02 - Song Two` 的前 3 个掩码字符都是 `**?`，判为相似 |

## 6. 用参考模型跑出的例子

参考模型（Python，按 §3–§5 逐条实现）没有放进仓库，下面的结果都由它算出。排序按 Vestra 现有的 `Intl.Collator("zh", {numeric:true, sensitivity:"base"})` 排列，这是对 `StrCmpLogicalW` 的近似。完整的测试表见 interaction-alignment.md 的 IA-PL-01「同类文件规则」。几个与直觉不同、但确实是 PotPlayer 行为的例子：

- `movie.mkv` 不会带入 `movie2.mkv`：名字里没有空格、`-`、`_` 或数字，前置条件不满足。
- `第０２集.mp4`（全角数字）什么也不带入：理由同上。
- `[字幕组] 进击的巨人 - 02` 会带入 `[字幕组] 鬼灭之刃 - 01`（S3 前缀相同），**不会**带入 `[其他组] 进击的巨人 - 03`。
- `Friends.S01E01` 会带入 `Friends.Special`（S1）；`Show.S01E01` 会带入 `Shower.Cam`（S4）。
- 打开 `01 - Intro.mp3` 时，同文件夹里所有 `NN - 标题` 形式的音频都会带入（S4 音频规则）。
- 全部模式只收同组文件：打开 `A02.mkv`，同文件夹的 `song.mp3` 不会加入。

## 7. 未确定的部分

- `0x180853230` 与打开流程中栈参数 +0x370 的确切含义：决定哪些「打开」会触发扩展（推断：网络路径、压缩包内文件、程序内部打开的列表项不触发）。
- 打开 URL 时的列表行为（见 IA-Q-02）。
- `job+0x294` 的作用。
- `StrCmpLogicalW` 的具体排序依赖 Windows 区域设置，中文按拼音还是按码位没有静态证据；Vestra 用 `Intl.Collator("zh")`。

## 8. 复现

```sh
mkdir -p $S/dl && curl -L -o $S/dl/PotPlayerSetup64.exe <metadata.json 的 installer.url>
shasum -a 256 $S/dl/PotPlayerSetup64.exe          # 4b9833adf53b78c2149fe13dd67f2504bf39ee30b45c03f09e5f2c25e43f9cfe
7zz x -o$S/x $S/dl/PotPlayerSetup64.exe
objdump -d --no-show-raw-insn --x86-asm-syntax=intel $S/x/PotPlayer64.dll > $S/w/dis.txt
grep -n "0x2825\b\|0x2826\b\|0x2827\b" $S/w/dis.txt     # 定位菜单处理 0x1803b8400 → main+0x6ac
grep -n "+ 0x6ac\]" $S/w/dis.txt                        # 读取模式的各个位置
analyzeHeadless $S/ghproj pot -import $S/x/PotPlayer64.dll -noanalysis \
  -scriptPath <脚本目录> -postScript Decomp.java out.c 180458b80 180453f70 ...
```

`Decomp.java` 是一个 20 行的 Ghidra 脚本：对给定地址建函数，再用 `DecompInterface` 输出 C 代码。
