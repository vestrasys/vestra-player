# Vestra Player

插件化的视频播放器：Rust 宿主 + Slint 原生界面 + TypeScript 插件，播放基于 libmpv。产品功能（播放栏、播放列表、画面调节、音频工作台、字幕、截图导出、NES 模拟器、YouTube、Bilibili、插件工坊（实验性））全部由插件实现，使用同一套公开插件 API。

```text
src/host/       Rust 宿主
src/runtime/    插件运行器（每插件一个 Node 进程）
src/sdk/        插件 API（src/sdk/plugin/）
src/plugins/    10 个产品插件
src/examples/   示例插件
doc/            文档（入口：doc/README.md）
```

## 运行

需要 Rust、Node ≥ 22.18、libmpv；导出和 YouTube 另需 ffmpeg、yt-dlp。

```sh
cd src && npm ci
cd host && cargo run --bin vestra-host -- --plugin ../plugins/playback --plugin ../plugins/playlist
```

`--plugin` 可重复，直接加载插件目录的源码。

## 文档

- 写插件：[doc/plugins/](doc/plugins/README.md)
- 项目方向与原则：[doc/product/](doc/product/direction.md)
- 开发流程（插件作者与 API 维护者的模拟社区）：[doc/process/](doc/process/community-workflow.md)
- 当前状态：[doc/status.md](doc/status.md)
