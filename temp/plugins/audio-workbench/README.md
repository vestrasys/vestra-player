# 音频工作台探针

实验版本：目录在 `src/` 不代表 API 稳定；参数、协议及界面可能与最终产品相差很大。

使用公共契约 probe-0.2 的兼容子集，在当前 probe-0.3 宿主回归，不需要迁移调用；见[基线登记](../../sdk/baselines/README.md)。

## 有限场景

1. 独立加载插件，初始不更改音频。
2. 面板实时调节 100 Hz / 1 kHz / 6 kHz 三段 EQ（±12 dB）、响度规范化、人声增强、±2000 ms 延迟。
3. 菜单的对白、音乐预设与面板共用状态；复位撤销自己的整条处理贡献。
4. 宿主拒绝配置时保留上一份成功参数；后续修改仍可正常操作。
5. 参数为插件实例级偏好，切换媒体继续生效；不绑定旧播放会话。不依赖播放控制、字幕或其他插件。
6. 停用移除自己的资源和动作订阅，其他 owner 的效果不受影响。

## Usage 与职责

```ts
// 实验公共 API；插件封装数值参数，宿主运行实际 FFmpeg 处理。
await ctx.effects.set({
  key: "audio-chain",
  kind: "audio",
  filter: "equalizer=f=100:t=q:w=1:g=3.000,loudnorm=I=-16:TP=-1.5:LRA=11",
});
await ctx.effects.remove("audio-chain");
```

人声增强是明确的频谱预设：120 Hz 高通、2.5 kHz 提升 4 dB、8 kHz 低通，不是 AI 分离或语音重建。响度规范化采用 FFmpeg 单遍 `loudnorm`（-16 LUFS、-1.5 dBTP、LRA 11），不承诺离线双遍母带处理精度。正延迟插入静音；负延迟裁去当前解码段开头并重设时间戳，不恢复已经播放的音频。后端重启/seek 时适用于新段，是实验边界，不是成熟时钟同步的替代品。

TS 不传 PCM，不运行 ffplay/ffmpeg 子进程，不引用其他插件；其 `.slint` 布局直接由 Slint 解释。宿主按进程身份限定 effect owner。

## 自动验证

```sh
node --test src/tests/probes/audio-workbench/*.test.ts
```

2026-10-17：6/6 测试通过。使用 `VESTRA_HOST=$PWD/.cache/probe-target/debug/vestra-slint-experimental1` 指定 maintainer 构建的 Rust 宿主。真实插件 Node worker 的 UI 动作设置效果，Rust 媒体任务采用当前处理链，输出 WAV，再由独立测量工具读取 PCM。

| 独立信号                                | 实测结果                                                    |
| --------------------------------------- | ----------------------------------------------------------- |
| 100 / 1000 / 10000 Hz EQ，相对原音幅度  | 2.7804 / 0.3608 / 1.9763                                    |
| 低电平音源规范化，RMS 倍率              | 7.6720                                                      |
| 人声增强，50 / 2500 / 15000 Hz 幅度倍率 | 0.1705 / 1.5733 / 0.1476                                    |
| 原始 1 秒脉冲，延后 250 ms              | 1.250 秒                                                    |
| 原始 1 秒脉冲，提前 250 ms              | 0.750 秒                                                    |
| 切换两份真实 MP4                        | 会话 ID 改变，预设保持                                      |
| 卸载音频插件                            | 只留下独立 CLI owner 的 `volume=0.5`，输出 RMS 为基线约一半 |

`--check-ui src/plugins/audio-workbench/manifest.json` 真正编译此 Slint 布局通过；这不是实际窗口截图。插件与本目录测试的 strict TypeScript 检查通过，TS/JSON/MD 通过 Prettier。没有设备实听或完整实时音画同步验收；音频实际播放的 ffplay 路径由宿主维护，不能用离线 WAV 结果冒充设备端播放质量验证。
