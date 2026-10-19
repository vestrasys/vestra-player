# 阶段 A 最小宿主（工作版）

本轮独立新实现；实验契约 `vestra.experimental/0.1`，尚非并行产品插件开发基线。

## 实际路线

Rust 持有单个长期存活的 libmpv，`rsmpv 0.2.1` 的安全 software render 接口将视频写入 Rust 所有的 RGB buffer；Slint 的 `Image::from_rgb8` 在宿主窗口绘制该帧。音频由同一个 libmpv 时钟/音频输出设施播放，不是另一条 FFmpeg/ffplay 播放链。第一方 crate 同时设置 `#![forbid(unsafe_code)]` 和 Cargo lint；依赖的底层 FFI 属成熟库边界。

播放控制和渲染在两个独立工作线程内执行，UI 线程只取最新帧并驱动 Slint。CPU mailbox 只保留最新一帧，不积压整个视频。参数提交不销毁/重建 libmpv，不重新 loadfile。宿主不创建外部 mpv 窗口。

选用 CPU 路线是阶段 A 的可验证安全桥接：wrapper 的 OpenGL 构造要求调用方 `unsafe`，目前不符合第一方约束。软件 renderer 官方明确提示较慢；此版固定 960×540 RGB 输出，CPU 解码、缩放、拷贝及 Slint 上传都消耗带宽，不能据此承诺 4K、HDR、零拷贝或高刷新率生产表现。后续可研究成熟安全 GPU 适配组件，但不得私加第一方 unsafe。图像缩放/纵横比由 mpv 的画布输出及 Slint contain 处理。

## 开发启动

macOS Apple Silicon 已实装 Homebrew Rust、mpv 与 Node（本轮 Node 因 libllhttp 依赖升级同步修复为可运行版本）。新机器需要：Rust toolchain（开发宿主）、动态 libmpv（本轮 0.41）、Node >=22.18（内置 TS type stripping）、npm。源码路径中的 `.cargo/config.toml` 配置 Homebrew arm64 lib 搜索目录；其他平台需安装并配置自己的 libmpv，尚未经本轮实测。

```sh
cd src
npm ci
npm run check
cd host
cargo build --locked
cargo run -- --plugin ../examples/capability-demo
```

演示媒体通过 `VESTRA_DEMO_MEDIA=/absolute/path/movie.mp4` 环境变量提供。宿主空载只显示原生视频画布和诊断状态；示例窗口是能力样例，不是产品播放栏、最终工具窗口规范或全局菜单设计。宿主不按示例 ID 决定行为。

插件是 manifest + JS/可直接剥离类型的 TS + Slint 文件，安装不需要 Rust 编译器。当前 load/disable 是包目录的运行生命周期；尚无复制安装库、签名/升级/回滚或节点运行时打包器。对终端用户仍需分发宿主可执行程序、兼容 libmpv 和 Node；本轮源码演示不是完成的安装包。

```sh
cd src
node scripts/verify.mjs
# 需先 cargo build；生成自有测试媒体和临时插件，通过实际进程边界验证。
```

## 当前明确缺口

- 仅单个普通 libmpv 会话；NES 自有时钟/视频音频生产、导出任务、处理链所有权、字幕异步隔离、Pi 工坊等均未实现。
- Slint UI 接入目前是插件独立原生窗口及 primitive properties/callbacks。嵌入播放栏/侧栏、全局菜单、快捷键路由、文件对话框、复杂模型、关闭事件尚待阶段 B/C 场景驱动补充。不能把独立窗口能力当作产品布局冻结。
- 插件 Slint 首次编译目前在 UI 线程发生，可能造成短暂停帧；频繁属性修改不重新编译。阶段 B/C 必须据实设计/修正首次打开工具与文件对话框行为。
- 现有 UI Timer 并不使用阻塞原生菜单；菜单展开期间真实播放连续性必须等产品菜单实现后实际验收，headless 帧前进不能替代该项证据。
- 播放调参为共享会话状态，不是插件所有的可撤销效果。此版不能承诺卸载处理插件恢复其他插件效果；后续处理资源 API 另行设计。
- 信任插件可访问 Node 和系统，不是安全沙箱。manifest 能力声明帮助发现与诊断，不构成恶意插件隔离。

## 官方依据

- [mpv render API](https://github.com/mpv-player/mpv/blob/master/libmpv/render.h)：软件渲染和调用线程约束。
- [rsmpv safe render 文档](https://docs.rs/rsmpv/0.2.1/rsmpv/render/index.html)：软件构造安全，OpenGL 构造需要 unsafe。
- [Slint Image](https://docs.rs/slint/1.16.1/slint/struct.Image.html)：共享像素缓冲区导入图像。
- [Slint interpreter](https://docs.rs/slint-interpreter/1.16.1/slint_interpreter/)：运行时编译插件 Slint。

许可证边界：实验 Cargo 元数据暂标 GPL-3.0-or-later（为与当前依赖开源选项兼容的实验选择，不替用户决定产品最终许可证）；Slint 开源 GPL 选项、Homebrew mpv GPL 构建及其依赖的实际再分发要求仍需打包时核对。本轮不要求购买商业许可，不宣称这些依赖可以闭源免条件分发。
