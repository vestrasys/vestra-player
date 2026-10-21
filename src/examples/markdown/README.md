# Markdown 示例（examples.markdown）

核心 Markdown 显示组件的演示（[S4 提案 §5](../../../doc/architecture/store-api-proposal.md)）。

- 菜单：示例 ▶ Markdown 示例…，打开一个窗口显示 `sample.md`。
- 窗口 `ui/document.slint` 只有一个 `MarkdownView`；`main.ts` 用 `plugin.ui.markdown(text, { base: 包目录 })` 生成属性值。
- 相对图片按包目录解析（`assets/icon.png`）；`https:` 图片由 SDK 下载到数据目录 `.vestra-cache/markdown/` 后显示；SVG 等不支持的格式显示替代文字；内嵌 HTML 不显示。
- 链接：`http(s)` 地址用浏览器打开（`system.openUrl`），`#锚点` 只显示屏幕提示。
- manifest 同时演示显示字段（`name`、`author`、`punchline`、`description`、`tags`、`icon`、`license`）与 `minPlayerVersion`。

开发：在 `src/` 下 `npm run plugin -- check|test|build examples/markdown`；运行 `cd host && cargo run --bin vestra-host -- --plugin ../examples/markdown`。
