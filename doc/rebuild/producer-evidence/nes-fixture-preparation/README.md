# Thwaite 合法 NES 素材：局部工具与 ROM 构建完成

2026-10-18。经 root 明确授权，在新系统临时目录局部构建官方工具与作者 ROM；未读取旧实现/缓存/ROM，未构建 Cargo、运行宿主/GUI/设备或模拟器，未写产品插件，未安装全局或产品依赖。当前**ROM 已构建且 iNES 头已核，未执行 NES 游戏验收**。

## 已固定的素材

官方 https://github.com/pinobatch/thwaite-nes ，本轮新 shallow clone 当前 revision `00e36745188bc165990f60eed6b093c3ce6ad0e3`。`source/` 保留该 checkout 全部源码与原配方/作者说明/许可（排除 .git 元数据）；原克隆目录、命令 stdout/stderr 和 revision 记录在 preparation-state/commands.json。

作者 README 说明整体 GPL-3.0-or-later，部分较通用源文件带更宽松的逐文件许可；`source/LICENSE.txt` 及所有源内版权原样保留。样本是独立测试材料，不混同产品插件许可。源码支持 NROM、标准手柄并含作者游戏音乐。

`src/nrom256.s` 声明 iNES magic、32KiB PRG、8KiB CHR、vertical mirroring/mapper0；源码声明现已由实际ROM头核实（见下节）。作者 `paldetect.s` 运行时探测 NTSC/PAL/Dendy，不能把该 ROM 说成 NTSC-exclusive；设计限定的后续测试应由固定核心使用 NTSC，实际结果另验。

## 本机工具与局部构建（授权后已执行）

PATH 与常见 /opt/homebrew/bin、/usr/local/bin 下均无 ca65、ld65。现有 /usr/bin/make、/usr/bin/cc、Python3 和 Pillow，可作局部构建前提。具体路径/版本原始输出已保留。

最小后续方案是另建新系统临时目录，clone 官方 https://github.com/cc65/cc65 ，固定 preparation-state.json 的 proposedLocalCc65Revision，再仅执行 `make -C <cc65>/src -j2 ca65 ld65`。已核官方当前 src/Makefile 有这两个独立目标，输出本地 bin/ca65 与 bin/ld65；不执行 make install、不改PATH或系统目录。随后 root 明确授权执行：新tmp中的 cc65 固定完整revision `6bd01e8a5b0e86389734fe22c679792214c01dc6`，仅 `make -C src -j2 ca65 ld65` exit0，未install或改全局PATH。完整官方工具源码（不含.git）在cc65-source.tar.gz，许可在cc65-LICENSE；实际两个工具复制到local-tools/，原始版本输出及完整构建诊断同目录保留。

Thwaite makefile 默认第一目标 run 会尝试启动模拟器，后续必须显式指定 `thwaite.nes` 或 `all`，不裸跑make。其 tools/dte 规则硬编码 `gcc -static`，macOS 可能不支持该链接方式；最小局部适配是先在临时checkout以 `cc -std=gnu99 -Wall -Wextra -DNDEBUG -Os -o tools/dte tools/dte.c` 新编译该数据工具（保留命令/任何失败），然后 `make thwaite.nes AS65=<local>/bin/ca65 LD65=<local>/bin/ld65 PY=<python3>`。不修改游戏逻辑或借现成ROM。本轮实际首次make exit2：`ld: library crt0.o not found`，原日志保留。随后按授权用上述无-static的本机cc命令编译原tools/dte.c（exit0），再次显式make thwaite.nes exit0；未改游戏源码或makefile。没有运行默认run目标。

## JSNES 官方接缝（未安装/执行）

官方 npm 精确包 `jsnes@2.1.0`，Apache-2.0，gitHead `cbad9f1db57dfb275b90ffa26f9c479c355d8d5c`；metadata/integrity 在 jsnes-package-metadata.json。无运行时 dependencies 字段；不代表已经安装或跑过游戏。官方对应 revision 的 README、NES 实现与声明已保留作接口核证。

公开 `new NES({sampleRate:48000,onFrame,onAudioSample})`、`loadROM(Uint8Array|ArrayBuffer)`、`frame()`、`buttonDown/Up(1,button)`、reset、toJSON/fromJSON 可满足有限接缝。实际 `.d.ts` onFrame 是 Uint32Array；README称ARGB/256×240，不能将其裸buffer主机字节序直接当RGBA8，需按固定实现确认通道并复制到获准slot。音频每回调left/right在[-1,1]，时钟由集成方管理，不能另用setInterval当权威时钟。fromJSON恢复前须先load同ROM，状态包含controllers，按设计须显式releaseAll，不能把保存的pressed keys带回新输入scope。

官方来源： https://registry.npmjs.org/jsnes/2.1.0 ，https://github.com/bfirsh/jsnes/tree/cbad9f1db57dfb275b90ffa26f9c479c355d8d5c ，https://github.com/pinobatch/thwaite-nes/tree/00e36745188bc165990f60eed6b093c3ce6ad0e3 。此准备不含真实NES画面/音频/性能/存档/producer验收。

## 最终产物与实际头

`thwaite.nes` SHA-256：`ee51cd9562f28195ba015d9857c6c4fc9bf67cdfb213e95f655e586b92195173`。实际40976bytes，头 `4e45531a020101000000000000000000`：legacy iNES、mapper0、32768bytes PRG、8192bytes CHR、vertical mirroring、无trainer、无battery；文件长严格等于16+PRG+CHR。头byte9 PAL flag=0（legacy NTSC标记）；作者runtime仍探测PAL/Dendy，不能称NTSC-exclusive。未启动任何模拟器，未声称实际画面、音乐、核心时序或存档兼容已验证。

`map.txt`、`thwaite.dbg`、完整命令/原静态链接失败及最终成功输出、工具源码/许可证和二进制均保留。`preparation-state.json`为结构化最终事实；`artifact-sha256.json`列工具源码归档、许可、本地工具和ROM摘要。Thwaite原始source/与source-file-sha256.json保留未改；该ROM是本轮作者源码新构建，不是下载预编译或商业ROM。
