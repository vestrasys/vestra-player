# Markdown 示例

这是 **核心 Markdown 显示组件** 的演示：*斜体*、~~删除线~~、`行内代码` 和 [Vestra Player 仓库](https://github.com/vestrasys/vestra-player)。点 [本页的「表格」一节](#表格) 只显示屏幕提示。

## 列表

- 无序列表
- 第二项，带 **粗体**
  1. 嵌套的有序列表
  2. 第二项
- [x] 已完成的任务
- [ ] 未完成的任务

## 代码块

```ts
const doc = await plugin.ui.markdown(text, { base });
window.set({ doc });
```

> 引用：Markdown 由播放器解析与绘制，插件只传入文本。
>
> > 嵌套引用

## 表格

| 元素 | 支持 |
| --- | --- |
| 标题、段落、列表 | 是 |
| 代码块、引用、表格 | 是 |
| 内嵌 HTML | 否（不显示） |

---

## 图片

本地图片（包内 `assets/icon.png`，按文档目录解析）：

![图标](assets/icon.png)

远程图片（由 SDK 下载到数据目录后显示）：

![Vestra](https://upload.wikimedia.org/wikipedia/commons/thumb/4/47/PNG_transparency_demonstration_1.png/280px-PNG_transparency_demonstration_1.png)

不支持的格式显示替代文字：![SVG 徽章](https://img.shields.io/badge/demo-svg-blue.svg)

<div>这段内嵌 HTML 不会显示。</div>
