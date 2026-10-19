# 阶段 A 工程规范

本轮独立新实现，工作契约 `vestra.experimental/0.1`。API maintainer 单点修改宿主、SDK、runtime 和契约；插件作者只写自身包。API 不稳定不代表可以悄改行为：阶段 B/C 收集能力请求、审查影响，形成同一设计/API/消费者基线后才并行产品实现。

## 职责和所有权

- Rust：公共播放设施、libmpv线程/内存、原生 Slint 对象、包运行生命周期和 RPC 路由。`backend.rs` 不含插件ID分支，控制与渲染线程独立；`plugins.rs` 管进程/包校验；`ui.rs` 直接接 Slint interpreter；`main.rs` 路由能力和所有者。
- TS：可信插件的策略和交互行为。`runtime/runner.ts` 是通用进程边界；`sdk/index.ts` 是公共客户端/类型。core 不识别某个插件名称执行业务。
- Slint：原生界面描述，直接使用其 widgets/layout 和 properties/callbacks。不给 UI 再造一个通用 JSON 布局系统。
- 独立插件不得 import 兄弟实现或共享私有对象，也不得以公共消息总线转发插件专属消息。共享 SDK、公开宿主状态和第三方库允许；多个插件通过同一会话设施协作。

第一方 Rust `forbid(unsafe_code)`，不得将 unsafe 搬进自建“第三方”小crate绕过。已选安全库负责底层 FFI；依赖仍需独立质量和许可证核对。软件 renderer 作为有限的真实桥接，不把它包装成4K/HDR/GPU验收。

每个实例具有唯一 generation，窗口/订阅归实例；停用/崩溃清理由宿主执行。回包、旧回调和旧进程 EOF 不得误伤新实例。Node 不是安全沙箱；资源所有权和包路径检查不等于权限隔离。

## 类型、反馈和错误

SDK public 类型和 Rust 校验必须同时更新。参数范围/单位、异步接受与实际完成、状态快照延迟、空值/能力缺失都要写明。不得用成功返回表示尚未执行的能力；后端异步 load 与渲染完成不能合并成一句“已播放”。长任务进入时必须定义取消和来源快照；当前瞬时方法的 timeout 不是取消。

UI/控制不得阻塞 Slint 线程等待后端；原生对象只在 UI 线程操作。后台渲染只交最新帧，不积压帧队列。libmpv 同步控制与 render 拆线程，保留单核心。插件停用的300ms等待/kill在后台执行，避免让菜单/主画布停止刷新。

不要添加产品插件业务来证明 API 存在。阶段 A `examples/capability-demo` 仅是独立消费者样例，后续产品必须根据统一设计重写入口和布局。

## 可执行检查

在 `src` 目录：

```sh
npm ci
npm run format:check
npm run check
npm run boundaries
cd host
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked
cd ..
npm run verify
```

- prettier / rustfmt：TS、JS和Rust格式。
- tsc strict/noEmit/erasableSyntaxOnly：公开SDK、runtime、样例和本轮测试类型检查；Node直接执行TS不应含不支持的转换语法。
- check-boundaries：检查当前新建 examples/plugins 的静态 import/export 和 literal dynamic imports，只允许相对路径落在自己的包或SDK；非literal动态import需人工审查。它不是安全沙箱，不能验证任意 fs/network 行为。
- Cargo：Slint/Rust依赖和构建、第一方unsafe禁止；clippy零warnings。
- verify：本轮新建合成媒体、独立进程与实际libmpv软件render，不依赖旧测试或报告。45次连续调参必须保持session、时钟和帧前进，验证pause/seek/错误与独立插件生命周期。它不是原生菜单/音频设备的物理验收。

首次实现同线程同步控制+render造成调参明显慢，正式改为控制/渲染分线程后重新记录数据。不能为通过测试减少连续性要求，不能把headless结果替代屏幕菜单展开期间的画面证据。

人工审查仍需检查：职责与命名、公共契约是否可读、是否有业务泄漏、不必要抽象、资源清理以及设计承诺的实际交互。新需求先提交用户场景/usage/所有权/失败取消语义与最小验收，再决定兼容扩展或破坏性迁移，不按探针名称加专属方法。
