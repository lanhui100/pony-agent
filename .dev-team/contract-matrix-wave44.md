# Wave-44 契约矩阵：前端健壮性收敛（PA-115 / PA-135 / PA-117）

- 作者：Protocol Agent（协议特派员，空桩/契约/NFR 唯一编写方）；基线 commit `ab54b06`；分级 **B**
- 写域：仅 `.dev-team/**`。本波**只读** `src/**`、`tests/**`（严禁修改）。
- **no new stubs**：本波是既有实现的缺陷修复，公共接口契约不变；不新增空桩文件、不新增 `TODO`/`unimplemented!`/占位导出。允许的唯一新增表面（均为可选、附加式）：① `useTimelineAutoScroll` options 的 `isDebugEnabled?: () => boolean`；② 调试开关谓词 1 个（导出，供组件与 hook 共用）；③ 开关 storage key 常量 1 个。三者语义由 PA-117 契约冻结，不得扩为公共 API。

## 0. 事实锚点（断言直接挂这些行）

| # | 事实 | 证据 |
| :-- | :-- | :-- |
| A1 | host-read 分支裸 await host 读，无界 | `src/stores/runtime.ts:2693-2694`（`case "host-read"` → `await this.loadSessionRuntimeViewState(...)`） |
| A2 | 会话切换分支已有同语义有界超时 | `src/stores/runtime.ts:2141` `void withTimeout(loadPromise, HYDRATION_TIMEOUT_MS, "加载对话").catch(...)` |
| A3 | `HYDRATION_TIMEOUT_MS = 15000`；`withTimeout` 超时 reject `"${label} 超时 (${ms}ms)"` 并在 settle 后清定时器 | `src/lib/runtime/constants.ts:15`；`src/lib/runtime/utils.ts:3-18` |
| A4 | 全 IPC 链路零超时（`measureHostRead`→`safeInvoke`→`invoke`） | `src/lib/runtime/utils.ts:80`；`src/lib/tauri.ts:76-86` |
| A5 | `initializeSessions` 外层 catch 已提供可见失败路径 + snapshot 回滚；finally 清 watchdog 与 `sessionOperation` | `src/stores/runtime.ts:2720-2729`（`:2722` 写 `初始化对话失败：`） |
| A6 | 3500ms watchdog 只清 `sessionOperation`，**不**取消 pending await ⇒ `initializeSessions` 永不 settle | `src/stores/runtime.ts:2639-2643` |
| A7 | 启动屏全屏遮罩受 `isInitializing` 控制，而它在 `await initializeSessions()` 之后才置 false ⇒ 挂起 = 永久启动遮罩 | `src/App.vue:162-164`；`src/components/AppStartupScreen.vue:34-38`（`fixed inset-0 z-50`） |
| A8 | `agentRuntimeStatus`：`initializing` → label「正在加载最近对话…」；`sessionError` 非空 → `mode:"error"` 且 label=该错误 | `src/stores/runtime.ts:472-482` |
| A9 | 组合态守卫无条件早退，位于 undo 分支之前 | `src/components/HomeWorkspace.vue:1400-1414`（`:1402` 守卫，`:1406-1413` undo） |
| A10 | Enter 提交分支（守卫的唯一必需服务对象） | `src/components/HomeWorkspace.vue:1416-1424`；keydown 绑在 textarea `data-testid="workspace-composer-input"`（`src/components/chat/WorkspaceComposer.vue:1017,1023`） |
| A11 | 撤销行为可观测：`handleUndoLastTurn`→`executeRollback`→`runtimeStore.checkoutHistoryNode` | `HomeWorkspace.vue:1379-1398,1340` |
| A12 | `console.info("[workspace-scroll]", payload)` 无开关；payload 由 `collectTimelineScrollMetrics()` 构造且 hook 侧再算一遍 | `HomeWorkspace.vue:211,196-200,155-157`；`src/lib/useTimelineAutoScroll.ts:147-181`（rect 读 `:153-156,175`）、`:184-190`（emit 先 `collectMetrics()` 再 `showDebug`） |
| A13 | 既有 debug 开关惯例：URL `?debug=1|runtime` 或 localStorage `"true"`，且**不缓存**（运行时即时生效） | `src/lib/runtime/utils.ts:19-32`；DEV 门例 `src/lib/frontend-flight-recorder.ts:549` |
| A14 | 既有测试依赖 debug 缓冲默认可用（测试/DEV 态必须默认开，否则回归） | `tests/HomeWorkspace.spec.ts:2405`（`latestScrollDebugEvent(...)` 必须 defined）；`tests/HomeWorkspace.spec.ts:2806` 为组合态 Enter 回归 |
| A15 | vitest 下 `import.meta.env.DEV` 可被确定性关闭：`vi.stubEnv("DEV", false)`（`DEV/PROD/SSR` 属 boolean env 组） | `node_modules/vitest/dist/chunks/test.DNmyFkvJ.js:3463-3467,3630-3636`；`.../chunks/init.d4hAcNdp.js:174-192` |

## 1. 契约总表

| 断言 id | 契约 | 冻结期望（可观测输出） |
| :-- | :-- | :-- |
| **PA115-T1** | 正常 host-read 不回归 | host view 正常返回时：`sessionId`=最新会话、`messages` 内容匹配、`sessionError===null`、`sessionOperation===null`、`agentRuntimeStatus.mode!=="error"`；`load_session_runtime_view` 恰好调用 1 次 |
| **PA115-T2** | host 读挂起 ⇒ 有界 settle + 可见失败 + 无迟到写 | ≤ `HYDRATION_TIMEOUT_MS+1000ms` 内 **resolve**（不抛）；`sessionError` 以 `初始化对话失败：` 开头且含 `超时`；`agentRuntimeStatus` = `{mode:"error", label===sessionError.trim()}` 且 label ≠「正在加载最近对话…」；`sessionOperation===null`；`phase` 回滚到快照；迟到 resolve 不改写 `sessionId/messages/phase`；随后再次 `initializeSessions()`（正常 mock）可恢复 |
| **PA135-T1** | 组合态 Ctrl/Cmd+Z 仍撤销 | `isComposing:true` 或 `keyCode:229` 时按 Ctrl/Cmd+Z ⇒ `checkoutHistoryNode` 调用 **1** 次、`preventDefault` 1 次、`submitTurn` **0** 次 |
| **PA135-T2** | 组合态 Enter 仍不发送 | `isComposing:true` / `keyCode:229` 的 Enter 与 Shift+Enter ⇒ `submitTurn` **0** 次、`checkoutHistoryNode` **0** 次；非组合态 Enter 仍提交 1 次、非组合态 Ctrl+Z 仍撤销 1 次 |
| **PA117-T1** | 默认关闭：零 console.info、零布局读数 | `DEV=false`+无开关（或开关 `"false"`）下触发 debug 事件序列 ⇒ `[workspace-scroll]` 前缀 `console.info` 调用数 **0**、`__ponyScrollDebugBuffer` 不增长 / `__ponyScrollDebugLatest===undefined`、`getBoundingClientRect` 调用数**严格小于**同序列启用态（差 ≥1） |
| **PA117-T2** | 显式开关可开可关、运行时不缓存 | localStorage `pony-agent.debug.workspace-scroll`=`"true"` 时（含 `DEV=false`）⇒ 输出恢复；=`"false"` 时（含 `DEV=true`）⇒ 关闭；URL `?debug=1`/`?debug=scroll` ⇒ 开启；同一已挂载实例内先关后开必须即时生效 |

## 2. 逐项详表

### 2.1 PA-115（host-read 无界 await）

- **缺陷事实**：`runtime.ts:2694` 裸 `await loadSessionRuntimeViewState()`；对照 `:2141` 已用 `withTimeout(..., HYDRATION_TIMEOUT_MS, "加载对话")`。A4 证明全 IPC 链路无超时 ⇒ 挂起后 `initializeSessions` 永不 settle（A6 watchdog 不取消 await），`App.vue:164` 永不执行 ⇒ A7 永久启动遮罩（`app-startup-screen` 常驻）；A8 的「正在加载最近对话…」在 3500ms 后被 watchdog 抹掉却**无任何错误**（静默空白）。
- **黑盒期望**：host 读必须有界（沿用 `HYDRATION_TIMEOUT_MS` 语义）；超时即走既有外层 catch ⇒ 可见失败（A5）且状态复位；不得静默降级成「空会话 idle」；迟到结果不得污染状态。
- **注入点/可测接缝**：`tauriMocks.mockSafeInvoke` 对 `load_session_runtime_view` 返回永不 resolve 的 Promise（保存 resolver 供 T2 迟到写断言）；`list_sessions` 返回 1 个未归档会话；`vi.useFakeTimers()` + `vi.advanceTimersByTimeAsync(15000)`（同 `tests/runtime-store.spec.ts:2316` 惯例）。只断言 store 公开状态与 `agentRuntimeStatus`，不 import 内部函数、不断言 `sessionHydrating` 私有实现顺序（`sessionHydrating=false` 为实现附带结果，可断言为「不得残留 true」但不得作为唯一断言）。
- **冻结断言**：PA115-T1、T2（见 §1；每条以 `expect` 收尾）。
- **Non-Goals**：不改 3500ms watchdog 职责；不给 `loadSessionCatalog` 等其它启动 IPC 加超时；不引入超时重试；不改 `deriveInitStrategy` / local-cache 分支语义；不实现「取消 IPC」（仅 bounded await）；不改切换/删除分支既有文案；正常路径耗时与调用次数不变。
- **NFR 映射**：`wave44_ui_robustness.hydration_read_timeout_ms=15000`、`settle_deadline_ms=16000`、`visible_failure_required=true`、`late_resolution_must_not_mutate_state=true`。`external_call_timeout_ms(3000)` 对本项 **N-A**：结构性理由——host 读是本地 Tauri IPC 水合，不产生外部网络出口，3000ms 外部调用上限不适用于本地渲染水合预算；沿用切换分支既有 15000ms 语义以保证一致性。

### 2.2 PA-135（组合态守卫吞掉撤销）

- **缺陷事实**：`HomeWorkspace.vue:1402` 无条件 `return`，使 `:1406-1413` 的 `isUndoShortcut` 分支在组合态不可达（A9）；守卫本意只服务 `:1416-1424` 的 Enter 提交（A10）。
- **黑盒期望**：守卫收敛为「仅拦截 Enter 提交」；组合态 Ctrl/Cmd+Z 仍撤销且恰好一次；组合态 Enter/Shift+Enter 不发送；非组合态 Ctrl+Z 与 Enter 行为不变；修饰键约束（`!shiftKey && !altKey`）保持；不得双触发（`handleWindowKeydown` 对可编辑目标不撤销）。
- **注入点/可测接缝**：mount HomeWorkspace（fixture 同 `tests/HomeWorkspace.spec.ts:4896`：2 条 historyNodes + 匹配 messages，使 `canUndoLastTurn` 为真）；`vi.spyOn(runtimeStore,"checkoutHistoryNode")`、`vi.spyOn(runtimeStore,"submitTurn")`；对 `[data-testid="workspace-composer-input"]` 派发 `keydown`，事件字段 `{key, ctrlKey/metaKey, shiftKey, altKey, isComposing, keyCode}`（jsdom 26 真实透传 `isComposing`/`keyCode`，见 `tests/acceptance/wave17-adversarial/ime-guard-adversarial.spec.ts:315-332`）。断言只挂「撤销/提交各发生几次」与 `preventDefault`，不断言分支顺序或函数内部变量。
- **冻结断言**：PA135-T1、T2。
- **Non-Goals**：不改 `handleWindowKeydown` 非编辑区全局撤销语义；不新增撤销二次确认/undo 栈；不改组合态下方向键/Esc/Shift+Enter 处理；不引入 compositionstart/end 本地状态机；不改撤销按钮与撤回进度实现；不新增「撤销进行中重复触发」断言（rollbackInFlight 护栏保持现状，不属本波验收面）。
- **NFR 映射**：`wave44_ui_robustness.ime_composition_guard.blocked_paths=["enter_submit"]`、`unblocked_paths=["undo_ctrl_z","undo_cmd_z"]`、`submit_turns_during_composition=0`、`undo_invocations_during_composition=1`。

### 2.3 PA-117（无开关的 debug 输出 + 强制布局读）

- **缺陷事实**：`HomeWorkspace.vue:211` 无条件 `console.info`，payload 由 `collectTimelineScrollMetrics()`（`:155-157` → `useTimelineAutoScroll.ts:147-181` 的 6+ 次 `getBoundingClientRect`）构造；hook `emit()`（`:184-190`）在调用 `showDebug` 前**又**算一遍 metrics ⇒ 每事件两次布局读数。缓冲写入 `:203-209`。A14 证明既有测试依赖「测试/DEV 态默认开」。
- **黑盒期望**：默认关闭（生产/非调试态）⇒ 零 `[workspace-scroll]` console.info、不写 debug 缓冲、**不为该通道构造含布局读数的 payload**（必须同时在 hook `emit()` 与组件 `emitTimelineScrollDebug` 两处短路，否则 hook 先算 metrics 仍违反零读数）；显式开关（优先级自上而下）：① localStorage `pony-agent.debug.workspace-scroll === "false"` → 关（kill switch）；② `=== "true"` → 开；③ URL `?debug=1|scroll` → 开；④ 否则 `import.meta.env.DEV` 真值 → 开，否则关。开关**运行时求值、不缓存**（对齐 A13）；开启态输出形状与现状等价。
- **注入点/可测接缝**：console 探针（`vi.spyOn(console,"info")`，按 `"[workspace-scroll]"` 前缀计数）；布局读数探针（beforeEach 包裹 `Element.prototype.getBoundingClientRect` 计数、finally 还原）；`vi.stubEnv("DEV",false)`（A15）+ `window.localStorage` + `window.history.replaceState({},"","/?debug=1")`；触发序列用 store 状态变化（如 `isSubmitting` true→false）产生 debug 事件，同 fixture 两态各跑一遍做差分。不断言 `collectMetrics`/`emit` 等内部符号。
- **冻结断言**：PA117-T1、T2。
- **Non-Goals**：不删除 `streamDebugState` / `pony-agent.metrics.stream-sessions` 落盘通道；不改 `HomeWorkspace.vue:1512` markdown 路径的采集（残留读数见盲区 B-117-1）；不引入日志框架或日志级别系统；不改 `[workspace-scroll]` 文案与 payload 字段名；不改滚动算法/阈值；不新增遥测上报。
- **NFR 映射**：`wave44_ui_robustness.debug_log_default_off.*`（见 `nfr-baseline-wave44.json`）。`logging.format=json / trace_id` 对本项 **N-A**：结构性理由——`console.info` 是开发者诊断输出，不是结构化日志出口（内核结构化日志面未变，无新增日志面）。

## 3. 全局约束（防假绿）

1. **测试不得依赖实现细节**：断言只可挂 ① store 公开状态/action 结果 ② 组件 `data-testid`（`workspace-composer-input`、`workspace-undo-button`、`agent-runtime-status-label`、`app-startup-screen`）③ 公开开关（localStorage key / URL 参数 / `vi.stubEnv("DEV")`）④ 浏览器原生 API 探针（`console.info`、`getBoundingClientRect` 计数）。禁止 import 私有函数、禁止断言内部变量名、禁止整对象 snapshot。
2. **不得无断言放行**：每条用例以 `expect` 收尾；禁止 `expect(true)`、禁止 try/catch 吞断言、禁止 `skip/todo`（仅盲区可登记并附工单）；禁止「运行不报错即绿」。
3. **不得为绿改测试**：契约与测试冻结后实现方只读；实现方不得修改 `tests/**` 或调低断言。
4. **回归底线**：`tests/HomeWorkspace.spec.ts:2405`（debug 缓冲默认可用）与 `:2806`（组合态 Enter 不提交）必须保持绿；`npm run test:unit` / `npm run typecheck` 必须 Exit 0。
5. **性能底线**：debug 关闭态为该通道新增开销 = 0 次 `getBoundingClientRect`、0 次 console 写入、0 次 buffer 分配。

## 4. 盲区（不做假断言）

- **B-115-1** [UNVERIFIED]：真实 Tauri IPC 挂起无法在 jsdom 复现，以 never-resolve mock 等价模拟（行为等价性假设，需真机冒烟佐证）。
- **B-115-2**：App 级断言（`app-startup-screen` 在 ≤16s 内消失）依赖既有 App stub 结构；若不可行，以 PA115-T2 的 `agentRuntimeStatus` 断言为准并登记降级，不得两者皆缺。
- **B-135-1** [UNVERIFIED]：真实 IME 组合态下浏览器可能把 `key` 报为 `"Process"`，此时撤销按 `event.key.toLowerCase()==="z"` 判定**不触发**；jsdom 注入 `key:"z"` 的断言不等价于真机，需人工真机验证（非门禁）。
- **B-117-1**：`HomeWorkspace.vue:1512` markdown 路径仍调用 `collectTimelineScrollMetrics()`，故关闭态「整页零布局读数」不成立；PA117-T1 的差分断言只在 debug 事件触发序列内比较，不外推。
- **B-117-2**：`?debug=1` 同时被 `isDebugLoggingEnabled()` 使用，两开关共享 URL 参数属**有意兼容**（A13），但不得据此把 runtime 日志通道纳入本波断言面。
