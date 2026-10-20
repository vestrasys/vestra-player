# NES 模拟器（nes）

功能：用 JSNES（`jsnes@2.1.0`，Apache-2.0，见 `node_modules/jsnes`）运行 iNES ROM，把每帧 256×240 画面（RGBA8）和 48 kHz 立体声 PCM 作为插件生成的音视频来源交给播放器。打开另一个 ROM 会先停止自己的旧来源；ROM 无效（非 iNES、超过 16 MiB、JSNES 不支持的 mapper）时显示错误并保留正在运行的游戏。

入口：`模拟器 → 打开 NES ROM…`（系统文件选择）、`模拟器 → NES 工具`（独立窗口 `nes.slint`：Open ROM… / Stop）、`模拟器 → 停止 NES`。Stop 只关闭仍属于本插件的来源；被别的来源替换时显示 “NES source replaced.”。打开途中 Stop 会撤销这次打开。

限制：本轮没有键盘输入和存档；不附带游戏 ROM；兼容性与时序以 JSNES 为准（NTSC，mapper 覆盖有限）。

开发（在 `src/` 下）：`npm run plugin -- check nes`、`build nes`（生成 `dist/`，版本取自 `manifest.json`）、`test nes`。测试用 `tests/homebrew.mjs` 现场生成自制 mapper-0 ROM；与宿主交互的测试使用 SDK 的 `TestHost`。
