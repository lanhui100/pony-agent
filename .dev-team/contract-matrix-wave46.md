# wave-46 契约矩阵（B 级 · 前端交互轨迹增强、局部错误降级与 Langfuse 属性收敛）

> Protocol Agent 冻结产物（契约/NFR 唯一编写方）。事实源：可观测性基座深度审计建议 + Wave-45 遗留。
> 写域：本波我**只**写 `.dev-team/`；`src/**`、`crates/**`、`tests/**` 归 Executor/Test Agent。
> **no new stubs**：禁止 `unimplemented!`/`todo!`/空函数体占位、禁止跳过用例与吞异常放行。
> 门禁纪律：测试先于实现红相锚定，必须通过 Exit 0 校验，严禁假自测。

---

## 0. 事实锚点（path:line）

| # | 事实 | 锚点 |
| :-- | :-- | :-- |
| F1 | 前端无局部错误边界（`onErrorCaptured` 命中为 0），组件崩溃直接冒泡整屏 | `src/main.ts:37` 全局 `errorHandler` |
| F2 | 前端仅 IPC 与性能诊断发面包屑，关键路由与交互行为缺面包屑 | `src/main.ts:17`、`src/lib/tauri.ts:82`、`src/lib/runtime/metrics-diagnostics.ts:46` |
| F3 | `App.vue` 核心页面状态切换（`currentPage` = `"home"` \| `"config"` \| `"telemetry"`）无交互轨迹 | `src/App.vue:19`、`:259`、`:279-285` |
| F4 | `ATTR_LANGFUSE_USER_ID` 常量已存在，但在 `begin_turn_trace` 中写死为 `None` | `crates/pony-agent-core/src/agent/otel/mod.rs:86`、`:1185` |
| F5 | `begin_turn_trace` 现有调用点未传递 `user_id`（仅接收 `session_id`, `turn_id`, `sequence`） | `crates/pony-agent-core/src/agent/runtime/turn_stream.rs:1292`、`turn_sync.rs:460` |
| F6 | 支持环境变量与配置级 `user_id` 注入（可映射为机器/客户端匿名标识或 env `PONY_OTEL_USER_ID`） | `crates/pony-agent-core/src/agent/otel/mod.rs:160-165`（环境变量模式） |

---

## 1. PA-161 · 前端交互与导航面包屑（UI Breadcrumbs）

| 缺陷锚点 | 黑盒期望 | 可测接缝 | 冻结断言 |
| :-- | :-- | :-- | :-- |
| F2/F3：页面导航与关键操作无面包屑，异常发生时无法还原用户前置交互 | 切换页面（`currentPage` 变更）或触发核心操作时，调用 `addBreadcrumb("navigation", "navigate_page", { from, to })`；面包屑队列最大保持 30 项并有界轮转 | Vitest 单测：触发页面切换或调用导航事件，断言 `getBreadcrumbs()` 包含 `category="navigation"`，data 包含目标页面信息 | **PA161-T1** |
| F2：交互面包屑数据脱敏防泄漏 | 面包屑 data 必须通过既有 `deepSanitize` 与 `truncateString`，严禁明文记录超长文本或 token | 同上：注入包含敏感 token 与超长字符串的 data，断言面包屑中敏感字段已被替换或截断 | **PA161-T2** |

---

## 2. PA-162 · 前端局部错误边界与优雅降级（UI ErrorBoundary）

| 缺陷锚点 | 黑盒期望 | 可测接缝 | 冻结断言 |
| :-- | :-- | :-- | :-- |
| F1：组件级崩溃导致整屏白屏 | 提供通用的 `ErrorBoundary.vue` 组件（使用 Vue 3 `onErrorCaptured` 钩子）；子组件抛出异常时：① 捕获异常并不再向上冒泡；② 自动调用 `reportError`（附带 `componentTag` 与 `trace_id`）；③ 渲染内联优雅降级 fallback 卡片（含错误提示与“重试/刷新”按钮） | Vitest 单测：挂载包裹了故意抛错子组件的 `ErrorBoundary`，断言父级无未捕获异常冒泡、降级卡片可见（`data-testid="error-boundary-fallback"`），`reportError` 恰好被调用一次 | **PA162-T1** |
| F1：重试与恢复机制 | 用户点击降级卡片的“重试”按钮时，错误状态重置，尝试重新渲染子组件 | 同上：模拟子组件修复并点击重试按钮，断言降级卡片消失，正常内容重新渲染 | **PA162-T2** |

---

## 3. PA-163 · Langfuse `user_id` 规范归因与注入

| 缺陷锚点 | 黑盒期望（wire 级） | 可测接缝 | 冻结断言 |
| :-- | :-- | :-- | :-- |
| F4/F5/F6：`langfuse.user_id` 恒为 `None`，导致语义轨无法按用户/设备归因 | 支持通过环境变量 `PONY_OTEL_USER_ID` 注入全局用户标识；非空且 trim 后：`agent.turn` 根 span 必须输出 `langfuse.user.id` 属性；未设置或纯空白串时：**绝对不得输出该键**（零空串占位） | 回环 OTLP sink 集成测试：设置 `PONY_OTEL_USER_ID="user-acc-test"` → `init` + `begin_turn_trace` + `force_flush` → 断言 body 包含 `langfuse.user.id` 与 `user-acc-test`；未设置时断言 `langfuse.user.id` 不出现 | **PA163-T1** |
| F5：签名向后兼容 | `begin_turn_trace` 保持既有调用签名兼容，或提供 `begin_turn_trace_with_user` 扩展，不破坏现有 15+ 处测试与调用点 | 现有全量 OTel 单元测试与集成测试（`otel_resource_turn_status_test` 等）零增量报错直接通过 | **PA163-T2** |

---

## 4. Non-Goals（本波非目标）

1. 不破坏 `OtelConfig` 的 4 字段既有公共定义与构造点。
2. 不强制改写已冻结的根 span 错误承接逻辑（`turn_error: ` 规则与 `agent.turn.outcome` 保持不变）。
3. 前端交互面包屑不拦截通用无意义 DOM 事件（如 `mousemove`、`scroll`），仅限显式导航与关键动作。
4. 不新增外部网络 IO 出口或第三方上报 SDK。

---

## 5. NFR 映射（详见 `.dev-team/nfr-baseline-wave46.json`）

| NFR 键 | 取值/判据 | 覆盖 |
| :-- | :-- | :-- |
| `external_call_timeout_ms` | 3000ms（继承） | 全波 |
| `logging` | format: json, trace_id: true | 全波 |
| `resource_release` | 错误边界重试清理 timer 与状态；OTel flush 正常 | 全波 |
| `wave46_ui_resilience.pa_161_breadcrumbs` | 导航面包屑 category="navigation"；队列有界 (<=30)；脱敏生效 | PA-161 |
| `wave46_ui_resilience.pa_162_error_boundary` | 捕获子组件异常，阻止冒泡，渲染 fallback UI，触发一次 reportError | PA-162 |
| `wave46_observability_semantics.pa_163_langfuse_user_id` | env 覆盖生效；非空即输出 `langfuse.user.id`；为空不输出该键 | PA-163 |
