# NES 核心独立验证（2026-10-18）

root 读取本轮新作者核心与原创 6502 homebrew 测试代码后，独立运行 `node --test /tmp/vestra-current-products/nes/tests/core.test.mjs`，实际退出0，2项测试通过。

两个 mapper-0 ROM 分别运行120帧，均产生95844个立体声采样帧，PCM能量1345.9468180751428；帧数据尺寸与RGBA透明度有效，两个ROM设定不同PPU背景色并产生不同SHA256。另检查通道转换及无效ROM拒绝。

这是实际 JSNES 2.1.0 核心执行，不是播放宿主或模拟producer回调。结果仅证明核心输出帧与有效非零PCM；不证明宿主接收、epoch同步、原生窗口、实际扬声器或完整输入交互。

本地完整日志 `/tmp/vestra-nes-core-independent.log`；退出码、测试文件和核心输出哈希记录 `/tmp/vestra-nes-core-independent.json`。首次原生安装拒绝了误带的 `.typecheck/node_modules` 符号链接，属于打包缺陷，维护者继续处理；未据此放宽加载器约束。
