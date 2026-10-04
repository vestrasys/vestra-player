# 实验 1：Rust Slint 宿主与 TypeScript 功能插件

> **这是实验版本，不是产品实现。所有接口、目录结构、进程安排和布局组合方式都是临时设计，与最终成品 API 可能相差很大；不提供兼容性承诺。**

本实验只评估 Slint：一个由 Rust 拥有的窗口，通过右键菜单添加插件。文件播放插件提供打开 MP4、播放/暂停与跳转；高斯模糊插件提供启停和强度设置。通过这两个实际插件，观察界面写法、跨语言通信和接入成本。

## 当前运行条件

- Rust **1.92 或更高版本**，Cargo；实验固定 Slint **1.18.1**。
- Node.js **22.18 或更高版本**，直接运行可擦除类型的 `.ts` 文件，不需要安装 npm 包。
- `ffmpeg`、`ffprobe`；启用音频时还需要 `ffplay`，且 FFmpeg 包含 `gblur` 滤镜。
- 桌面图形环境，以及首次构建时下载 Cargo 依赖所需的网络。

[Slint 1.18.1 的官方 workspace](https://github.com/slint-ui/slint/blob/v1.18.1/Cargo.toml) 声明最低 Rust 版本为 1.92。本实验没有静默使用旧版本或放宽第一方 Rust 的 `unsafe` 禁令。

## 启动与操作

在本目录执行：

```sh
./scripts/run.sh
```

启动时，窗口没有内置播放操作栏：

1. 在视频区域右键，选择“添加插件”。
2. 选择 `plugins/file-playback/manifest.json`，窗口底部出现插件自己的操作栏。
3. 点击“打开 MP4”，选择一个本地视频。播放、暂停、跳转均通过这个插件完成。
4. 再右键添加 `plugins/gaussian-blur/manifest.json`。
5. 在滤镜插件中启用高斯模糊，设置强度并应用，对比处理前后的画面。
6. 添加 `plugins/playlist/manifest.json`，右侧出现播放列表；添加文件后可按条目播放。
7. 点击侧栏“隐藏”，再通过右键菜单“显示播放列表”展开。隐藏与展开不重新编译宿主，不替换窗口。

可以生成一个便于观察模糊效果的测试视频：

```sh
./scripts/make-sample.sh
```

默认输出 `samples/sample.mp4`，包含动态测试画面和测试音频，不纳入版本管理。

关闭播放器会清理插件子进程与播放子进程。插件标准输出专用于通信；日志写入标准错误。

## 这次怎样接入 Slint

```text
Rust Slint 窗口
  ├── 视频 Image
  ├── 文件播放插件的原生 .slint 组件
  └── 高斯模糊插件的原生 .slint 组件
             │ Slint callback / global property
             ▼
      Rust 实验事件桥
             │ JSON 消息；不传视频帧
             ▼
      Node 中的 TS 插件逻辑
             │ 播放命令、滤镜参数、界面属性更新
             ▼
      Rust 播放后端 → FFmpeg
```

**窗口与界面由 Rust 的 Slint interpreter 创建。** 本实验没有使用 Slint 的 Node GUI 绑定，没有让 Node 接管主窗口。

插件作者直接编写现成的 `.slint` 界面语言，使用 Slint 自带控件、布局和样式。实验没有另外定义按钮、布局、位置等 GUI 语言。TS 使用小型实验 SDK 处理界面动作、调用播放能力，以及更新该插件自己的 Slint global 属性。

## 添加插件的真实机制与边界

添加插件时，Rust 读取 manifest，将插件导出的 Slint 组件 import 到一份新的根文档，再交给官方 interpreter 编译。新的组件树替换旧窗口，播放后端、插件进程及已缓存的界面属性保留。

因此，运行中添加插件会经历**重新组合并编译界面、替换窗口**，可能出现短暂停顿或窗口闪动。它不是稳定 API 下的任意运行时组件插入，也不能据此认定 Slint 已解决完整的插件热插拔。

本实验按添加顺序放置插件面板。面板的高度、布局、控件和样式由 `.slint` 原生语言表达，没有设计通用 `placement` 协议。它也没有解决任意面板竞争空间、复杂 docking 或插件覆盖整个宿主等产品级问题。

播放列表使用一个专门的实验右侧挂载点，manifest 的 `ui.region: "sidebar"` 仅用于这次验证，不是最终布局协议，也不代表允许任意插件挤占窗口。侧栏布局、控件和样式仍使用原生 Slint，未引入按钮注册 API。隐藏使用条件组件，释放侧栏占用的布局空间；列表及当前页存放在 TS 进程中，重显后保留。

## 播放列表窗口实验

```sh
./scripts/playlist-experiment.sh
```

脚本会实际打开窗口，以静音样本自动执行插件加载、播放、暂停、跳转、加载侧栏、隐藏与重显、再加载滤镜，成功后退出。它走真实 Slint 回调、Node 插件 RPC 和 FFmpeg 后端；文件对话框在此自动验证模式下返回指定样本路径，没有自动操作原生文件选择器。

结果写入 [evidence/playlist-window-report.json](evidence/playlist-window-report.json)，并通过 Slint 公开窗口快照接口保存 [展开](evidence/playlist-shown.png) 与 [隐藏](evidence/playlist-hidden.png) 截图。运行需要桌面环境。首次加载新组件仍会重建窗口；本实验没有采用 Slint 的内部动态嵌入接口，也没有实现完整插件安装管理器。

## 插件如何写

每个插件目录包含：

```text
manifest.json  # 声明入口、Slint 文件和导出名称
ui.slint       # Slint 原生界面，包含自己的 global 状态与 action 回调
index.ts       # 行为逻辑
```

`index.ts` 的基本形态：

```ts
// 仅为本实验的使用方式，最终 API 可能完全不同。
export async function activate(ctx: PluginContext) {
  ctx.onAction("open", async () => {
    const path = await ctx.dialogs.openMedia();
    if (path) await ctx.player.load(path);
  });

  ctx.onPlayback(async state => {
    await ctx.ui.patch({
      "is-paused": state.paused,
      position: state.position,
      duration: state.duration,
    });
  });
}
```

实际可阅读的两个插件是本实验的主要评估对象：[文件播放](plugins/file-playback/index.ts)、[高斯模糊](plugins/gaussian-blur/index.ts)。Slint 字段、回调和实验 SDK 的准确约定以源文件为准。

插件进程拥有 Node 的运行时权限；独立进程用于分开运行与故障处理，本实验没有实现权限沙箱。TS 可以使用 Node 文件系统等通用能力，但视频帧不经 TS。

## 播放与滤镜的实验取舍

当前机器已安装 FFmpeg，没有 mpv。本实验使用 **FFmpeg 子进程**解码、缩放到预览尺寸并输出 RGB 帧，Rust 通过 Slint 的安全图像接口呈现。高斯模糊由 FFmpeg 的 **CPU `gblur`** 执行，TS 只传递启停和 sigma 参数。

这是 GUI 与插件集成的验证后端，不是最终 mpv 后端，也不是 GPU shader 性能验证。视频帧仍有 CPU 解码/处理、拷贝和纹理上传成本，不能据此评价成品性能。

播放后端使用固定预览尺寸与帧率。暂停、跳转或改变滤镜时，按当前位置重建处理管线；暂停时改变滤镜只请求预览帧。音频使用独立 `ffplay` 进程，属于粗略同步，不是生产级音视频同步。具体实现和已验证边界见 [findings.md](findings.md)。

## 检查

```sh
./scripts/check.sh
# 工具链与依赖齐备后：检查宿主及动态组合的 Slint 源码
./scripts/check.sh --gui
```

检查分为：

- 真实 TS 插件子进程的动作、请求/响应和界面状态更新。
- 不依赖 Slint 的 Rust 后端测试，以 FFmpeg 生成并处理真实 MP4。
- 当工具链与依赖可用时，Cargo 对完整宿主进行检查，以及 Slint 界面编译检查。

检查脚本与 [findings.md](findings.md) 会明确区分已执行成功、缺少依赖而未执行、以及尚未人工验证的项目。源码存在不等于桌面实验已经跑通。

## 评估这个实验时要回答的问题

1. `.slint` 加 TS 的插件写法是否容易理解，是否允许所需的布局与样式？
2. 添加插件时重新构建窗口的代价是否可接受？若不可接受，Slint 当前的稳定接口是否足够？
3. 界面状态、异步动作和播放状态之间的通信是否清晰？
4. 两个插件共同运行、操作失败和关闭播放器时，资源能否正确收回？

运行通过后，再决定是否继续 Slint 的动态组件集成研究。实验中的字段、进程模型及后端不自动升级为项目架构决策。
