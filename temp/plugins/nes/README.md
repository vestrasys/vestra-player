# NES 来源探针（实验）

本版本用于架构验证，最终 API 可能相差很大；不是商业游戏包。对应验收 N1/N2/G1。使用公共契约 probe-0.2 的兼容子集（包含 API-006），在当前 probe-0.3 宿主回归，不需要迁移调用；见[基线登记](../../sdk/baselines/README.md)。

## 独立运行与操作

在仓库根目录执行 `npm ci --prefix src/plugins/nes --ignore-scripts` 安装精确锁定的 JSNES 2.1.0（Apache-2.0）。它执行真实的 6502、PPU 和 APU，不用预生成画面或音频冒充模拟器。

启动 `src/scripts/run.sh`，加载本目录 manifest。菜单「文件 / 模拟器」打开 `.nes` ROM；「模拟器」提供暂停、状态保存/恢复、关闭会话。取消选择不改变会话；无效 ROM 在替换现有来源之前失败。

点击视频区域获得键盘焦点：方向键、Z=A、X=B、Enter=Start、Shift=Select。面板由原生 `.slint` 定义，不依赖其他插件，也不占用播放列表的单一 sidebar 插槽。面板隐藏、切换和重建不释放会话；真正停用插件才回收来源。

可生成测试自制 ROM（CC0，无商业资源）：

```sh
node --input-type=module -e "import {writeFileSync} from 'node:fs'; import {testRom} from './src/tests/probes/nes/rom.ts'; writeFileSync('/tmp/vestra-test.nes',testRom());"
```

自制 ROM 用实际 6502 指令设置 PPU 背景色、读取 `$4016` 的 A 键并驱动 APU pulse channel。按 Z 改变画面颜色，释放恢复；用于可观察的输入/帧证据，不代表完整游戏兼容性测试。

## 生命周期与性能边界

- ROM 执行时钟属于插件。60Hz 单个异步 tick，仅一个 present 在途；暂停等待在途提交结束再更新宿主权威 paused 状态。
- 来源分辨率 256×240，RGB24 base64；44100Hz interleaved stereo PCM16LE。Rust 持有呈现资源，TS 作为内容生产者产生帧；这一低分辨率 IPC 不构成普通视频处理或 GPU 零拷贝方案。
- 外部来源 `duration:null,path:null`，没有伪造视频时长或 seek。宿主返回 sessionId，所有提交绑定该会话。
- 状态文件是 JSNES 序列化加版本和 ROM SHA256。恢复先构建候选模拟器，坏状态/其他 ROM 不破坏活跃实例；保存通过原子帧边界快照后异步写盘，拒绝覆盖已有文件。
- 停用清理 timer、等待在途提交、解除订阅、关闭本插件来源；不引用或直接调用任何其他插件。
- 原生音频实际提交给宿主 ffplay；自动测试检查 PCM 非零及消费计数，不把该检查宣称为人工实听或声卡延迟验证。

## 自动验证

```sh
node src/node_modules/typescript/bin/tsc -p src/tsconfig.json
VESTRA_HOST="$PWD/.cache/probe-target/debug/vestra-slint-experimental1" \
  node --test src/tests/probes/nes/*.test.ts
```

`engine.test.ts` 执行真实自制 ROM，断言 A 键前后像素变化、非零 PCM，并逐帧比较存档恢复后的像素与 PCM SHA256。`dialog.test.ts` 只验证 ROM 过滤及取消契约。`integration.test.ts` 启动真实 Rust headless 宿主与 Node 插件，验证来源、真实传输、输入、暂停稳定、磁盘存档恢复和卸载。

GUI 编译与实际窗口证据由独立整体验收单独记录；headless 测试不替代 GUI 验收。API 反馈见 [API_REQUESTS.md](API_REQUESTS.md)。
