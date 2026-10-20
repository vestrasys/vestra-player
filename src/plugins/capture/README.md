# 截图（vestra.capture）

功能（决定 D3，对齐 PotPlayer）：
- `保存当前画面`（Ctrl+E）：不弹窗，把当前原始画面（PNG）直接存进截图文件夹，文件名为「片名_播放时间.png」，完成后屏幕提示「帧已保存为 …」。
- `当前画面另存为…`：弹出保存对话框，保存执行命令那一刻的画面。
- `复制当前画面`（Ctrl+C）/ `复制实际画面`（Ctrl+Alt+C，F03）：截取后导出 PNG 到 `<tmpdir>/vestra-capture-<pid>/`，用 `plugin.clipboard.writeImage` 放进剪贴板，再删除临时文件、释放快照。剪贴板总是 PNG。与单张截图共用「一次一张」：上一张未完成时提示「上一张截图仍在处理」，不排队。
- `保存实际画面`（Ctrl+Alt+E）/ `实际画面另存为…`（F04）：`processed: true`，包含插件画面效果；`实际画面包含字幕`（勾选，默认开）决定是否带字幕。
- `截取为 PNG / JPEG / BMP`（单选）与 `截图设置…`（F07）：格式、JPEG 质量（1–100，默认 90）、截图文件夹、「条件允许时储存到播放文件夹」。适用于单张、另存为、连拍。JPEG/BMP 先导出隐藏的 `.<名字>.vestra-tmp.png`，再用 `sips` 转换；失败时保留 PNG 并提示「无法转换为 JPEG，已保存为 PNG」。另存为时输入 `.png/.jpg/.jpeg/.bmp` 按输入的扩展名决定本次格式。
- `截图文件夹…`：选择截图文件夹。设置（`folder`、`format`、`quality`、`screenSubtitles`、`besideMedia`）用 `plugin.storage.json("settings")` 保存在插件数据目录的 `settings.json`，下次启动恢复；读坏的文件由 SDK 改名为 `settings.json.bad-<时间戳>`，单个字段无效时只有该字段用默认值；没有设置或文件夹已不存在时用「图片」文件夹。
- `打开截图文件夹`（F09）：用 `plugin.system.openPath` 在 Finder 打开截图文件夹（不存在时先创建）；开启「储存到播放文件夹」且当前是本地文件时打开视频所在文件夹。无媒体也可用。
- `连拍…`（Ctrl+G）：独立对话框（保存设置 / 截图设置 / 其他设置 / 信息，底部「打开存档文件夹」「截取当前画面」「开始」），2–20 张、间隔 0.1–10 秒，按媒体时间 t0 + i × 间隔采样，写入截图文件夹下唯一的 `Capture-*` 子目录。
- `导出片段…`：对话框（范围 / 输出设置 / 信息），导出 MP4，可选一条源内音轨或无声音，可包含画面效果与字幕。
- `导出任务…`：任务列表，每项可取消、打开输出（`plugin.system.openPath`，连拍打开其文件夹）、重试（失败或取消后保留的输入最多五分钟，过期后用当前来源重试）。

入口：`视频 → 截图 ▶`（420–438，分组 capture-source / capture-screen / capture-export / capture-format / capture-name / capture-folder）。窗口：`ui/burst.slint`、`ui/clip.slint`、`ui/tasks.slint`、`ui/settings.slint`，打不开时显示 `ui/error.slint`；`ui/parts.slint` 是共用的分组框与标签。

行为要点：快照以用户操作时看到的来源为条件，保存对话框打开期间来源变化则什么都不截。已接受的任务冻结输入：一个运行、最多四个排队。从不覆盖已有文件，排队中的任务也不会分到同一个文件名（自动 `name (n).ext`）。关闭窗口不取消任务；停用插件时取消未完成任务并释放快照。

代码：`main.ts`（菜单、窗口）、`form.ts`（冻结输入并排队、重试）、`tasks.ts`（队列、取消、保留、格式转换、打开输出）、`copy.ts`（复制到剪贴板）、`settings.ts`（设置的默认值与逐字段校验）、`model.ts`（解析、校验、命名）、`sips.ts`（JPEG/BMP 转换，唯一保留的系统命令；测试用 `tests/fakes.ts` 替换）。剪贴板、打开文件夹、设置文件用 SDK 通用能力（manifest 能力 `clipboard`）。

限制：只支持可独立重读的本地有限时长视频；没有数值进度（只有任务阶段与已保存张数）；编码器可用性以任务实际失败呈现。

开发（在 `src/` 下）：`npm run plugin -- check capture`、`build capture`、`test capture`。
