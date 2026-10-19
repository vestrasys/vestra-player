# 实验 API 请求

## API-003：具名文件选择器（已批准）

字幕文件不是媒体视频，`dialogs.openMedia()` 的固定过滤器无法正确表达选择 SRT。建议 `ctx.dialogs.openFile({extensions:['srt'],title:'加载字幕'})`，宿主持有原生对话框，取消返回 null。headless 测试通过公共开发 harness 配置选择结果。maintainer 已将契约、SDK 和宿主实现同步；NES/导出也可复用，插件无特殊宿主身份分支。

## API-007：受管文件资源（probe-0.3 候选迁移）

独立验收 REV-005 证明：快照保存字幕 filter 字符串，却继续引用可被改写或删除的缓存文件，不满足不可变处理快照。maintainer/root 批准显式破坏性迁移；probe-0.2 固定快照保留，不静默改写旧契约。

字幕改用 `filter: "subtitles=filename={{file:captions}}:..."`，并声明 `files: { captions: cache }`。宿主注册时复制资源，效果、快照和任务持有副本，最终引用释放后清理。插件仍负责自身原始缓存，不解析宿主资产路径。

影响：字幕效果注册与 filter helper；导出测试中的字幕 fixture 同步迁移，导出插件接口不变。root 按维护者精确 usage 做窄迁移（原作者暂无并发名额），待维护者实现后重新执行字幕、导出和独立快照/卸载回归；当前不标为已验证发布。类型、业务场景、像素和生命周期断言不能为兼容新实现而削弱。

### 作者迁移复核与回归

原作者已复核 root 的 `files:{captions:cache}` 与 `{{file:captions}}` 窄迁移，符合 probe-0.3 声明，不引入插件之间的依赖或私有 API。重新运行三项字幕测试、strict TS 与单插件 Slint 编译通过；新增真实 RGB 对比证明缓存改写与生产者卸载后，其他 owner 的旧快照保持原结果。完整独立验收仍由验收角色执行；这些消费者通过不等于候选基线整体已发布。
