# 专辑封面与空闲画面（vestra.visualizer）

依据：doc/product/feature-requirements.md「专辑封面与空闲画面（`visualizer`）」J11、H12；第 3b 轮任务单 §4.1 D 行、§4.3。插件只负责「视频区没有画面时画什么」：挂一个背景层（`panels.mount("backdrop")`），什么时候显示由播放器决定（`panels.backdrop.state.effective`），画什么由插件按 `player.state.phase` 与 `stats().video` 自己判断（裁决 R-U3：没有 `mode`）。

## 菜单与设置页

| 菜单路径 | order | group |
| --- | --- | --- |
| 声音 ▶ 纯音频时显示专辑封面（勾选） | 530 | visual |
| 设置 ▶ 外观 ▶ 空闲时显示徽标（勾选） | 845 | settings-logo |
| 设置 ▶ 外观 ▶ 选择徽标图片… | 846 | settings-logo |
| 设置 ▶ 外观 ▶ 恢复默认徽标（用默认徽标时灰，原因「正在使用默认徽标」） | 847 | settings-logo |

没有默认快捷键。选项窗口「外观」分类里注册页面「徽标与专辑封面」（id `logo-cover`，order 20，`ui/page.slint`）：两个开关（与菜单同一份设置）、当前徽标预览与「选择图片…」「恢复默认」、封面缓存的条数与大小和「清除封面缓存」。

## 画什么（`main.ts`）

| 来源 | 画面 |
| --- | --- |
| `phase: empty`（启动、F4 关闭、列表结束） | 空闲：徽标（H12），开关关闭时纯色 |
| `ready` / `ended`，`stats().video === null` | 纯音频：封面与标签（J11），开关关闭时纯色 |
| `ready` / `ended`，有视频 | 纯色（背景层本来也不显示） |
| `loading`（G-16b） | 加载界面：74×74 转圈（12 个刻度，每 50 ms 一格，`Timer` 只在加载中运行），下方粗体 15 px「正在打开：<来源名>」（`Theme.text-dim`）；来源名为文件名，链接为「主机/路径末段」，超过 60 字截断。不受空闲、封面两个开关影响 |
| `error` | 回到加载前的画面（核心此时保持背景层上一状态） |

- 每个 `sourceId` 就绪时读一次 `stats()`（同一来源重新加载后也再读）；读不到时用背景层自己的 `effective`。之后背景层又变为显示而当前判为视频时再读一次。
- 换来源（`sourceId` 变化）立即中止进行中的 ffprobe / ffmpeg（`tools.run` 的 `signal`），晚到的结果丢弃。
- 图片（封面、自定义徽标）只在背景层挂载成功之后才写入，坏图片不会让挂载失败；播放器报告图片错误（`invalid` / `missing`）时插件重新检查两张图，坏的那张退回占位图或品牌徽标。

## J11 专辑封面（`covers.ts`）

- 封面按顺序取：内嵌封面（ffprobe 显示 `attached_pic = 1` 的流，ffmpeg 取一帧）→ 同文件夹 `cover` / `folder` / `front` 的 `.jpg|.jpeg|.png`（不分大小写）→ 占位图（品牌徽标 40% 不透明，框同封面）。
- ffprobe 一次读标签与封面流：`-show_entries format_tags:stream=index,codec_type:stream_tags:stream_disposition=attached_pic`。标签键不分大小写（FLAC 写 `TITLE`），容器标签优先，没有时用音频流的（Ogg / Opus）。没有标题时用文件名；第三行「专辑 · 年份」（年份取 date 的前四位）。
- 封面一律经 ffmpeg 缩到 ≤ 1024 边长的 JPEG（`-f image2pipe -c:v mjpeg`，由插件原子写入），减少播放器解码内存；没有 ffmpeg 时文件夹封面原样复制（PNG/JPEG 且 ≤ 4096×4096、16 MB，按文件头判断）。ffprobe、ffmpeg 各 10 秒超时。
- 缓存：`<dataDirectory>/cache/covers/<sha1(realpath+size+mtime)>.jpg|png`，同目录 `tags.json`（`{ version: 1, entries: { key: { title, artist, album, date, cover, bytes, lastUsed, folder? } } }`）。上限 200 MB、30 天，按 `lastUsed` 淘汰；启动时删除 `tags.json` 没有引用的文件。文件夹封面记下其大小与修改时间，变了重新读。只缓存 ffprobe 给出答复的结果（没装 ffprobe、超时下次重试）。
- 网络来源：只显示占位图与地址最后一段，不运行 ffmpeg。
- 界面（`ui/backdrop.slint`）：封面边长 = 视频区短边 60%，最大 480，且不超过放完文字后的剩余高度；圆角 `radius-panel`、1px `Theme.line` 描边、柔和阴影；非方形封面保持比例居中。文字最宽 80%，超长省略；高度 < 200 时只留标题。新封面 300 ms 淡入（Slint `states` 的进入动画），场景切换 200 ms 淡入淡出。

### 已知行为：带封面的音频由 mpv 显示

mpv 把**内嵌封面**和**同文件夹的 cover.jpg / folder.jpg 等**（mpv 默认 `cover-art-auto=exact` 加 `cover-art-whitelist`）都当作视频轨选中，此时 `stats().video` 非空、背景层 `effective = false`，视频区显示的是 mpv 渲染的封面图（铺满、无标签文字），不是本插件的画面（裁决 3.2 第 8 条，不算缺陷）。所以实际上本插件的音频画面只在**没有任何封面**的音频文件上出现（显示占位图与标签）。内嵌与文件夹封面的提取、缓存代码照常保留并有测试，供核心改为把封面交给背景层时直接生效（见第 3b 轮报告中的 API 请求）。

## H12 空闲徽标（`logo.ts`）

- 内置徽标：`ui/logo.png`（由 `src/assets/brand/logo.png` 缩到 512×512，Retina 下 256 点仍清晰），Slint `@image-url`，不依赖运行时图片。边长 = 短边 30%，限制 64–256；下方 12px「Vestra」（13px，`Theme.text-dim`），视频区高度 < 120 时隐藏；徽标后有一圈很淡的 `Theme.playing` 径向光晕，徽标 92% 不透明，深色背景上不刺眼。
- 自定义徽标：`dialogs.openFile` → 扩展名（png/jpg/jpeg）与文件头检查，> 16 MB 或 > 4096×4096 提示「图片过大（最大 4096×4096、16 MB）」，不是 PNG/JPEG 提示「只能使用 PNG 或 JPEG 图片」→ 复制为 `<dataDirectory>/logo.png|jpg`（按真实类型，临时文件加改名，删掉另一种扩展名的旧副本）→ `ui.image`。原文件之后删掉不影响。
- 启动时自定义徽标读不到或不合格：用品牌徽标，提示一次「徽标图片无法读取，已使用默认徽标」，并把设置改回默认（之后不再提示）。

## 设置

`<dataDirectory>/visualizer.json`（`plugin.storage.json`）：`{ "audio": { "on": true }, "idle": { "on": true, "logo": null | "logo.png" | "logo.jpg" } }`，逐字段校验，坏字段用默认值。

## 开发

在 `src/` 下：`npm run plugin -- check|build|test plugins/visualizer`。`ui/*.slint` 中没有硬编码颜色（全部 `Theme` token，配色切换时跟随）。
