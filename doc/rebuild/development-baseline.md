# 插件开发工作基线 development-2026-10-18.1

2026-10-18：**已发布为本轮产品实现工作基线**。wire版本仍为`vestra.experimental/0.1`，工作基线标识不是多版本协商承诺。root已重算[最终快照](package-evidence/author/final-source-hashes.json)14项SHA256全部匹配；安装/回退作者和独立有限闭环已通过，NES特化清理及受影响检查完成。允许产品编码，不代表全部能力或产品验收通过。

## 当前来源与使用边界

SDK以当前 `src/sdk/index.ts` 及其导入模块为准，实际宿主路由以当前源码为准。`api.md`是阶段A历史说明，不能当作后续候选能力不存在的证明。`api-candidates.md`、`media-api-candidate.md`、`subtitle-api-candidate.md`、`producer-api-candidate.md`描述本轮新增能力，但可能保留早期状态；使用前与SDK及本表核对。

| 能力 | 代码入口 | 已有依据与开工用法 | 实际缺口如何处理 |
| --- | --- | --- | --- |
| 普通播放与来源仲裁 | `sdk/source.ts` | 真实libmpv；优先source.submit/state/operation，普通文件使用open intent，pause/seek/rate按来源capabilities | 不增加playlist业务到宿主；旧playback方法不继续扩展，与新source重叠的迁移由维护者明确 |
| 命令及菜单 | `sdk/commands.ts` | 注册领域菜单、状态更新及回调；入口遵守B1与新站点设计补丁 | 不从一个插件执行兄弟的私有命令来协作；入口与产品状态由作者实现 |
| 主窗/工具/底栏/侧栏 | `sdk/index.ts`的ui/window/dialogs | 普通Slint；已有真实组合实证；set/show不重编译 | 原生窗口最新CUA绑定失败，真实点击保待验，不阻止Slint与插件代码实现；不是所有工具原生验收已过 |
| 画面与音频处理 | `sdk/processing.ts` | 有限typed资源及owner生命周期，运行时处理与暂停预览有有限实证 | 预设、滑条节流、交互映射留作者；按真实插件路径补缺，不先验证所有参数组合 |
| 字幕 | `sdk/subtitles.ts` | 分块上传/选择/样式/操作查询有有限实证 | 解析与翻译策略归插件，不把生成译文或站点字幕获取放核心 |
| 截图与片段导出 | `sdk/media.ts` | snapshot/jobs；真实PNG/MP4和若干取消/旧来源/尾部反例已有材料 | 作者实现产品任务界面与输入检查，未覆盖变体列产品验收，不重跑全部旧套件才能编码 |
| 外部音视频与输入 | `sdk/producer.ts` | 二进制AV/有界队列/公共pause；真实JSNES音画作者实验已运行 | 本轮NES仅要求能跑；键盘原生交互暂缓，通用输入保留，Reset/LoadState已合并为通用Reconfigured；欠载如实保留 |
| 插件生命周期与安装 | `plugins`调试客户端与`packages`托管客户端 | load accepted与activation ready必须分开；stage/activation模块已存在 | 作者headless/原生Slint协议及独立两轮已验证有限试载清理、持久安装、失败保旧和升级回退；已通过本轮工作准入；原生点击及中断恢复故障注入仍未覆盖 |
| 网络站点来源 | 当前source仅open字符串/openProvider，并无通用解析提供者注册契约 | YouTube/Bilibili设计需提出最小公共路由usage；TS站点插件负责解析、下载 | 新能力由站点真实切片提出并维护；播放列表与站点插件不能互相import/读对象/专用通信 |
| Pi | 工坊插件自带依赖 | 真实Pi1.3.0+mock streamFn+TypeScript5.9.3 JS诊断修复前提已跑 | 不增加模型调用到core；产品工坊、离线依赖打包和界面仍待作者实现 |

## 包管理实验容量

安装操作持久记录暂定最多512条，满时在新事务prepare前明确busy，已有记录继续精确重放/查询。不淘汰后悄悄重新执行旧requestId；本轮不新增无限历史或跨重启请求epoch平台。该有限容量须在实际SDK文档与验收报告中注明，不宣称P1的过期识别已经实现。

## 发布时只补实际检查结果

- [x] 安装/回退有限闭环与失败恢复已有作者及独立实际证据，接口见[包管理候选](package-api.md)，最终修订检查仍待补。
- [x] 公共NES特化审查与语义清理：root词边界扫描当前host/sdk无NES/JSNES/ROM/存读档/按钮映射；双Shift物理标识保留为通用键码。Reset/LoadState已合并Reconfigured，受影响检查由维护者完成后补结果。
- [x] 最终快照14项SHA256核匹配；Clippy0、TS0（含plugins/**/*.ts）、2项契约变更测试0、feature build0、修改文件format0与boundaries0。证据在package-evidence/author，早期失败与有限未验项保留。
- [x] 开发目录`src/plugins/<id>/manifest.json`无version、entry main.ts，用调试plugins.load；生产dist包包含version/生产JS入口走packages。插件只import type Host，运行时消费activate(host)，dist无需运行时import仓库SDK。普通来源优先source；旧playback仅设备volume/mute等尚无替代的调用，不扩展重复来源接口。
- [x] 发布development-2026-10-18.1；立即分派基础播放与播放列表产品切片，作者独占各自目录，维护者继续处理真实使用请求。

产品完整验收矩阵不是上述发布清单。公开缺口不是最终通过；发布后维护者持续修复实际消费者暴露的问题，必要破坏性变更按工作流统一迁移。

## 开发启动与本轮交付

开发宿主：`src/host/target/debug/vestra-host --plugin /absolute/path/src/plugins/<id>`。需要重建时由维护者统一执行`cargo build --locked --manifest-path src/host/Cargo.toml --features producer-audio-probe`；作者不并行Cargo或操控同一GUI。插件TS检查/生产JS构建由作者自己运行，Slint编译/实际宿主集成与维护者协调。不要读取旧temp、Git历史或旧插件实现。

作者先交“入口→逻辑→公共调用→可观察结果”的可运行产品切片；当前基线未实现通用站点URL resolver，不阻塞本地播放/列表。YouTube条目先保存原URL，网络提供者集成由明确API请求补齐，不读站点插件。NES只需真核心运行音画，键盘/存档暂缓。完整已确认产品范围为十个组件；本基线不是成品API冻结。
