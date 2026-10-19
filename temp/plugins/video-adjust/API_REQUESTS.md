# 画面调节探针：实验 API 请求

本文件及插件均为实验，接口与成品可能相差很大。

## VA-001：带所有权的视频处理资源（批准，契约 v0.1）

维护者决策：`ctx.effects.set({key,kind:'video',filter})` / `remove(key)`，使用实验 FFmpeg expression escape hatch，而非每种效果增加 core 专名 API。插件验证全部数字后构建表达式。能力为 `effects.video`。实际 owner 由宿主绑定。

- 用户操作：调整亮度、对比度、饱和度、色相；选择预设；复位。
- 现有缺口：`setGaussian` 只有一个全局开关，既无法表达色彩处理，也无法隔离处理所有权。
- 建议 usage：`ctx.video.effects.set({ id: 'adjustments', kind: 'color-adjust', brightness, contrast, saturation, hue })`，以及 `remove('adjustments')`；名称以维护者落盘契约为准。
- 范围建议：brightness [-1,1]，contrast [0,2]，saturation [0,3]，hue [-180,180]；非有限和越界输入拒绝，不静默截断。
- 状态：TS 持有最近成功配置；core 按插件 owner + 本地资源 ID 持有实际处理链。请求失败不覆盖最近成功配置；宿主在停用/崩溃后清理此 owner。
- 共存：另一个插件的高斯处理资源不受本插件复位、停用影响。不读取其他插件配置。
- 能力缺失：激活失败且提供清楚诊断，不虚报画面已调整。配置请求失败应展示错误并保留最近成功状态。
- 破坏性：应替代历史单例滤镜接口，不要求保持实验兼容。
- 验收：真实 FFmpeg/Rust 宿主生成像素，调节后变化；同时存在第二个独立处理资源，移除本资源后第二资源仍存在且像素效果保留。

## VA-002：多入口命令（批准，契约 v0.1）

维护者决策：manifest `commands:[{id,title,menu:string[]}]` 路由到既有 `onAction(id)`，无需另起 UI 控件 API。

- 用户操作：通过视频菜单的亮度增减、预设入口，以及另一处复位入口操作同一状态；Slint 面板也使用同一动作。
- 建议 usage：注册稳定本地 action ID、标题、菜单路径。宿主路由回插件已有 action 回调；无需创建另一套按钮/布局 API。
- 生命周期：菜单属于注册者；停用即撤销；插件不向兄弟插件发消息。
- 验收：命令与面板 action 得到同一处理配置；菜单路径不全部挂在同一子菜单。

## VA-003：真实宿主验收入口（批准，API-002 实现中）

维护者决策：headless Rust 宿主 + 真实 Node worker；`media.start({processing:'current',format:'image',...})` 在任务启动时快照 live owner chain；`effects.list()` 公开只读资源描述验证撤销。这可证明实际链生效，避免测试只重复传递插件自己的 filter。

- 需要通过公开测试宿主启动真实插件与 Rust 处理路径，避免直接 import 宿主内部文件或仅 mock RPC ack。
- 用例：单独激活、命令触发、错误恢复、停用清理、两个 owner 共存、输出 RGB/PNG 像素证据。
