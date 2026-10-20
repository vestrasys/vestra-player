# 章节、书签与区段（vestra.marks）

依据：doc/product/feature-requirements.md「章节、书签与区段（`marks`）」B21、B23（B22 进度条标记在第 3b 轮）。来源只通过 `player.state` 观察，跳转用 `player.seek`，循环用 `player.setLoop`；不和其它插件通信。

## 菜单与快捷键

| 菜单路径 | order | group | 默认绑定 |
| --- | --- | --- | --- |
| 播放 ▶ 章节/书签 ▶ 章节/书签列表… | 260 | marks-list | H |
| 播放 ▶ 章节/书签 ▶ 添加书签 | 261 | marks-edit | P |
| 播放 ▶ 章节/书签 ▶ 上一书签/章节 · 下一书签/章节 | 262 · 263 | marks-edit | Shift+PageUp · Shift+PageDown |
| 播放 ▶ 章节/书签 ▶ 上一章节 · 下一章节 | 264 · 265 | marks-edit | Ctrl+PageUp · Ctrl+PageDown |
| 播放 ▶ 章节/书签 ▶ （动态）章节，单选 = 当前章节 | 10000 + i | marks-chapters | |
| 播放 ▶ 章节/书签 ▶ （动态）书签 | 20000 + i | marks-bookmarks | |
| 播放 ▶ A-B 区段循环 ▶ 设定起点 · 解除起点 · 起点后退/前进 0.1 秒 | 270–273 | ab-a | `[` · `{` · Ctrl+[ · Alt+[ |
| 播放 ▶ A-B 区段循环 ▶ 设定止点 · 解除止点 · 止点后退/前进 0.1 秒 | 274–277 | ab-b | `]` · `}` · Ctrl+] · Alt+] |
| 播放 ▶ A-B 区段循环 ▶ 区段整体后退/前进 0.1 秒 | 278 · 279 | ab-both | Ctrl+Alt+[ · Ctrl+Alt+] |
| 播放 ▶ A-B 区段循环 ▶ 循环当前章节/书签区段 · 区段循环 开/关（勾选） | 280 · 281 | ab-toggle | Alt+\ · `\` |
| 播放 ▶ A-B 区段循环 ▶ 循环次数 ▶ 无限 / 1 / 2 / 3 / 5 / 10 次（单选） | 282–287 | ab-count | |

播放 ▶ 记忆播放位置（history）占 250–254，所以本插件从 260 起。微调命令 `repeat: true`。菜单每个路径段最多 80 字节（宿主限制），章节与书签的菜单文字按字节截短并加 `…`，重名加 ` (2)`。

## B21 章节与书签（`chapters.ts`、`bookmarks.ts`、`actions.ts`、`menu.ts`）

- 章节：每个来源第一次就绪时读一次 `player.chapters()`（核心不推送）；换来源立即清空。
- 菜单最多各列 50 个章节、50 个书签，之后一行「更多章节…（共 N 项）」/「更多书签…」打开窗口。变化在 150 ms 内合并后整体重建子作用域。
- 上一个以 `position − 1` 为基准，下一个以 `position + 0.5` 为基准；书签与章节同一时间时按章节提示。
- 书签：±0.5 秒内不重复，每个文件最多 500 个，默认标题 `书签 N`。本地文件按 realpath、链接按原文记录，另记文件大小，大小不同时窗口顶部提示「文件已改变，书签时间可能不准」；之后编辑该文件的书签即采用新大小。
- `bookmarks.json`：`{ version: 1, media: { <标识>: { size, modified, items: [{ time, title, created }] } } }`，500 ms 去抖写入，超过 2000 个文件时按 `modified` 淘汰最旧的。

## 「章节/书签」窗口（`list.ts`、`ui/marks.slint`）

380×420（最小 320×300）。搜索框按标题过滤；列表按时间排列章节（暗色、只读）与书签；单击选中、双击跳转；选中书签时下方显示「时间」（`h:mm:ss.d`）与「标题」输入框，回车或离开输入框时提交；底部 `添加书签` `移除` `清空书签`（`dialogs.confirm` 确认）。换来源时丢弃选择与未提交的输入。

## B23 A-B 区段循环（`loop.ts`）

- A、B 只在插件里；A、B 都有且开关为开时 `setLoop({ a, b, count? })`，否则 `setLoop(null)`（核心已无循环时不发）。开始循环时位置不在 [A, B) 内先跳到 A。微调 100 ms 去抖后提交；条件提交遇 `stale` 重试一次。
- 开关的勾选 = `state.loop` 是否非空：别的插件设的循环也显示为开，`\` 关闭它。核心在次数用完后把 `state.loop` 置空：提示「区段循环结束」，保留 A、B。换来源清空 A、B。
- 循环次数即核心的 `count`（回跳次数：2 = 区段共播 3 遍），存 `marks.json` 的 `abCount`。

## 开发

在 `src/` 下：`npm run plugin -- check|build|test plugins/marks`。延迟集中在 `store.ts` 的 `timing`，测试里缩短。
