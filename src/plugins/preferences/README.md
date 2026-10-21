# 首选项（vestra.preferences）

依据：doc/product/feature-requirements.md「首选项（`preferences`）」J04、H10。选项窗口与配色都属于播放器（`plugin.settings`、`plugin.appearance`），本插件只提供入口和自己的「外观」页，不和其它插件通信。

## 菜单

| 菜单路径 | order | group | 绑定 |
| --- | --- | --- | --- |
| 设置 ▶ 选项… | 830 | settings-main | F5 |
| 设置 ▶ 外观 ▶ Vestra（单选，默认） | 840 | settings-look | |
| 设置 ▶ 外观 ▶ PotPlayer 经典（单选） | 841 | settings-look | |

## J04 选项…

`settings.show()`：打开播放器的「选项」窗口（或带到前面），显示上次的页面。窗口、分类树、记住页面都由播放器负责。

## H10 外观页（`ui/appearance.slint`）

- 页面 id `appearance`，分类「外观」，标题「外观」，order 10（同一分类的其它插件页面排在后面时用更大的 order）。宽 440。
- 两行配色单选：单选圆点、名称、一句说明，右侧 120×40 预览（视频区底色、底栏底色、时间文字、进度轨道与填充、强调色圆点）。整行可点。
- 「恢复默认」：回到 Vestra；已是默认时置灰。
- 切换（菜单或页面）后 OSD「配色：Vestra / PotPlayer 经典」；选当前值不发送、不提示；连续点击排队，只切换一次。别的插件切换时菜单勾选与页面跟随，不提示。失败时 OSD 显示原因，配色不变。
- 预览颜色：`Theme` 只有当前配色，所以两套预览色由 `themes.ts` 以数据（`{ r, g, b }`）传给页面，页面里用 `rgb()` 画，`.slint` 中没有颜色字面量。数值抄自播放器的配色表（ui-alignment §2.1），播放器改配色表时要同步改这里。
- 没有圆角选项，也没有配色列表接口（裁决 R-S2）：配色 id 固定为 `vestra`、`potplayer`。配色由播放器保存，本插件停用后保持。

## 开发

在 `src/` 下：`npm run plugin -- check|build|test plugins/preferences`。
