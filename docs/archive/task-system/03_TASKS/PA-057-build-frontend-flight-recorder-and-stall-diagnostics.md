# PA-057 建立前端飞行记录仪与卡顿诊断体系

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- 当前 change 已归档：
  [2026-06-19-build-frontend-flight-recorder-and-stall-diagnostics](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-19-build-frontend-flight-recorder-and-stall-diagnostics>)

## 背景
当前前端卡顿诊断仍主要依赖零散 `console`、临时局部埋点与事后代码推断，无法稳定回答以下问题：

- 卡顿发生前后的完整时序是什么
- 卡顿是业务计算慢、Vue 响应式级联、Markdown 渲染、滚动/动画还是 GC pause
- 卡顿发生时当前 `sessionId / turnId / phase / message规模 / DOM规模` 是什么
- 同一次卡顿是否总发生在 `turn:completed`、`MarkdownRenderer` 或 `HomeWorkspace` 某条链路上

现有链路虽然已经有：

- [`debugLog`](</C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts>)
- `__ponyStreamMetrics`
- [`record_stream_debug_metrics / load_stream_debug_metrics`](</C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/lib.rs>)

但它们仍然偏“最后状态”或“单点日志”，缺乏一套可持续、可落盘、可按会话回放的前端飞行记录仪机制。

## 目标
建立一套系统化的前端埋点与卡顿诊断方案，使 Pony Agent 在前端发生卡顿时，能够自动保留足够的结构化证据，用于事后定位、对比与回放。

## 输出
- 新 OpenSpec change：`build-frontend-flight-recorder-and-stall-diagnostics`
- 一套前端 flight recorder 的正式 spec / design / tasks 文档
- 覆盖事件模型、stall 检测、冻结现场快照、Tauri 持久化、查询/导出、采样与开销控制
- 至少一轮基于 `opencode / deepseek-v4-flash-free` 的三维并行 spec 审核
- 根据采纳意见调优后的文档版本

## 范围边界
- 本卡先收口“诊断与观测体系”，不直接承诺解决所有卡顿根因
- 本卡负责定义统一 recorder、事件模型、持久化与导出机制
- 本卡可以定义首批关键链路埋点，但不要求一次性覆盖全项目所有组件
- 本卡优先使用 Tauri/SQLite 承载持久化，不再依赖浏览器 console 或 WebView localStorage 作为主诊断来源
- 本卡不以浏览器自动化或手工 DevTools 操作为前提

## 验收标准
- spec SHALL 定义统一的前端 trace event 模型与最小必填字段
- spec SHALL 定义主线程 stall 检测策略，且不依赖单一浏览器专有 API
- spec SHALL 定义 stall 触发后的冻结现场快照内容与大小边界
- spec SHALL 定义 Tauri 侧持久化方案与最小查询/导出能力
- spec SHALL 定义首批必须埋点的关键链路，而不是泛泛要求“以后多打点”
- spec SHALL 定义采样、限流、ring buffer、批量 flush 与开销控制策略
- spec SHALL 明确前端 UI 偏好缓存与正式诊断持久化的职责边界
- 至少完成一轮使用 `opencode / deepseek-v4-flash-free` 的 3 个不同角度并行只读审核

## 当前进展
- 已完成前端 flight recorder 主实现：
  - [frontend-flight-recorder.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/lib/frontend-flight-recorder.ts)
  - [runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)
  - [MarkdownRenderer.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/MarkdownRenderer.vue)
  - [HomeWorkspace.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeWorkspace.vue)
  - [HomeSidebar.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeSidebar.vue)
  - [App.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/App.vue)
- 已完成 Tauri / SQLite 持久化、查询与导出主实现：
  - [frontend_diagnostics.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/frontend_diagnostics.rs)
  - [control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/control_plane.rs)
  - [lib.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/lib.rs)
- 已完成最小运维消费路径：
  - 侧边栏 recorder 摘要
  - JSON / Chrome Trace 导出入口
  - workspace / sidebar 双入口手动 stall smoke 注入
- 已完成验证与回归：
  - `npx vitest run` 通过，`13` 个文件、`194` 条测试
  - `npx vue-tsc --noEmit` 通过
  - `npm run build` 通过
  - `cargo check --manifest-path src-tauri/Cargo.toml -p pony-agent-core --target-dir target-check-core` 通过
- 已新增定向测试：
  - [frontend-flight-recorder.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/frontend-flight-recorder.spec.ts)
  - [HomeWorkspace.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/HomeWorkspace.spec.ts)
  - [HomeSidebar.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/HomeSidebar.spec.ts)

## 本轮实现补充
- `HomeWorkspace` 已补首批正式热路径埋点，覆盖 `turns`、`latestTurnSignature`、`syncStreamingPresentationState`、滚动跟随与用户滚动意图。
- recorder `flush` 失败回排已从逐条重新 `enqueue` 收紧为批量前置回排，降低失败时的自放大噪音。
- 新增可重复的前端 stall smoke 注入入口，方便在不依赖浏览器操作的情况下制造卡顿并抓取结构化证据。
- **启动冻结分析与修复（本轮新增）**
  - `SessionStore::new()` 同步加载 11MB SQLite 阻塞主线程 7.7s → 改为 `OnceLock` 后台线程初始化
  - `requestAnimationFrame` 在 WebView2 下被节流 → 启动链 yield 改为 `setTimeout(0)`
  - flight recorder 集成导致循环依赖 + flush retry storm → 模块级惰性初始化 + 指数退避 + initialized 守卫
  - Rust 端 6 个诊断命令全部 async + `spawn_blocking`
  - git hook `powershell` 5.1 无法解析 Unicode → 改为 `pwsh`
  - 提交：`f6aab80`、`20f8a56`

## 剩余说明
- `opencode / deepseek-v4-flash-free` 的 3 路 spec 审核已完成并被采纳。
- 本轮再次进行 3 路代码审核时，需显式规避 `session continue` 跑偏与快照锁冲突；若工具未稳定产出有效结论，应如实记录，不将其充当正式 findings。
- 当前仍存在测试环境里的 `v-motion` 未注册 warning，但不影响本卡实现、构建或测试通过结论。

## 下一步动作
1. 用新增的 stall smoke / 导出入口对真实长对话继续采样
2. 基于导出的 session/turn/window 证据继续定位真实卡顿根因
3. 如需归档 OpenSpec change，可在下一步执行 archive

## 当前卡点
- 无实现阻塞；当前进入“真实运行采样与根因分析”阶段

## 断点续跑提示
继续前先看：

- [src/stores/runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)
- [src/components/HomeWorkspace.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeWorkspace.vue)
- [src/components/MarkdownRenderer.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/MarkdownRenderer.vue)
- [src-tauri/src/lib.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/lib.rs)
- [2026-06-16 turn completion freeze diagnosis](</C:/Users/HUAWEI/Documents/pony-agent/docs/analysis/turn-completion-ui-freeze-diagnosis-2026-06-16.md>)
