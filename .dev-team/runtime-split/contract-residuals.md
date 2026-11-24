# runtime store 拆解残余：死路径与归属裂缝登记表（B280）

> 口径：本表是 Wave 2（B279/B280/B277 等）的**可执行清单**，不是叙述性报告。
> 每条必须有证据（文件:行 或可直接重跑的检索命令）与明确状态；无证据的猜测不进表。
> 登记日期：2026-11-22 · 关联 ADR：`2026-11-22-runtime-store-contract-residuals`、
> `2026-11-22-runtime-store-cross-session-timer-guards` · 关联条目：B279 / B278 / B277。

## 状态图例

| 状态 | 含义 |
|---|---|
| 已修 | 本 PR 改了代码并有回归测试/门禁 |
| 已登记 | 语义确认属既有行为，只登记不动（改动需另立决策） |
| 待修 | 已确认是缺陷/死路径，但改动面大，排入后续波次 |

## 一、死路径（可安全删除，删除不需新决策）

| # | 项 | 证据 | 状态 |
|---|---|---|---|
| R1 | `streamFlushFrameId`（rAF 调度）为纯死码：只有 `cancelAnimationFrame` 读取，无任何 `requestAnimationFrame` 赋值点 | `rg -n "requestAnimationFrame" src/` → 无命中；字段原仅出现在 `state.ts` / `types.ts` / `sessions.ts` / fixtures | 已修（本 PR 删除字段、分支与 13 个 fixture 键，84 → 83 键） |
| R2 | `restoreSessionRuntimeSnapshot` 里的 `cancelAnimationFrame` 分支随 R1 失效 | `src/lib/runtime/sessions.ts`（原 171-173 行） | 已修（同步删除） |

## 二、归属裂缝（同一语义有两个家 / 守卫缺口）

| # | 项 | 证据 | 状态 |
|---|---|---|---|
| R3 | trace 节流回调无归属守卫：定时器存活期内切换会话，旧 pending timeline 落到新会话 | `telemetry.ts` 原 `scheduleThrottledTraceTimeline` 回调无 `sessionId`/`activeTurnId` 校验 | 已修（回调捕获 owner 并校验；回归 `timer-session-guard.spec.ts`） |
| R4 | `switchSession` 不 cancel stream flush、不落定 pending trace：切走后旧 setTimeout 把增量写进新会话 | `orchestration.ts` 原 730 行前后无 `cancelStreamFlush` / `clearPendingTraceTimeline` | 已修（切走前 flush pending → cancel → 清缓冲；缓冲已先存入 `runningSessionMap`） |
| R5 | `applySessionSnapshot` 会话落点不清理旧会话定时器 | 原先仅 `sessionId` 赋值，无定时器清理 | 已修（`sessionId` 变化时 cancel + clear；同会话刷新不打断） |
| R6 | rIC 延迟 timeline 恢复（PA-089 优化）无会话归属守卫：空闲回调晚于下次切换会覆盖新会话 | `orchestration.ts` 原 305 行 `runLowPriorityTurnWork` 回调无校验 | 已修（校验 `this.sessionId === ownerSessionId`） |
| R7 | Pinia `$patch` 通过 `turn-events.ts` 局部别名 `TurnEventsThis` 绕过契约类型；`$reset`/`$subscribe` 无任何裁决 | `turn-events.ts` 原 66 行局部类型；全仓 6 处生产 `this.$patch` | 已修（契约收敛到 `types.ts` 的 `RuntimeStoreBuiltins`；机械守卫 `runtime-builtins-contract.spec.ts`） |
| R8 | 会话状态恢复有两个家：store action `applySessionSnapshot` 与 `src/lib/runtime/sessions.ts` 的 `restoreSessionRuntimeSnapshot` 双实现 | 两处各自维护字段清空面；R1/R2 正是双写导致的死键残留 | 待修（合并需先确认 host 侧持久化契约，改动大） |

## 三、已登记不改（既有语义，禁止顺手统一）

| # | 项 | 证据 | 状态 |
|---|---|---|---|
| R9 | `resetSessionRuntimeState` 不清 `latestExecutionCheckpoint` / 队列 / 工作区树 | `session-switch-reset.spec.ts` 已冻结该不对称 | 已登记 |
| R10 | `$patch` 在测试 Arrange 阶段被大量使用（harness `seedStore` 等 10+ 处） | `__tests__/**` | 已登记（契约允许：测试 Arrange 白名单） |

## 四、待 B277（Wave 2 等价测试）收口

40 个未覆盖 action 的等价测试缺口按 store 域拆分补齐；本表不重复列 action 清单，
真源为用户于 B277 记录的 fingerprint `frontend::stores::runtime::test-coverage-gap`。

## 修前约束（推翻 golden fixtures 的口径）

1. 本表 R1/R2 已删除 state 键，**13 个 golden fixture 已同步**；后续再删字段必须先跑
   `npx vitest run src/stores/runtime` 全绿，再改 fixtures，禁止反向（先改夹具再改代码）。
2. `contract-surface.spec.ts` / `concurrency-lifecycle.spec.ts` 的键清单是冻结面：
   任何键增删都要同时改 `types.ts` 的 `FrozenStateKeys` 与两处测试字面量，三者缺一即红。
3. `.dev-team/runtime-split/fact-map.md` 是拆解前的历史测绘快照，**不改写正文**，
   只在顶部追加修订注记（本次已注记 84 → 83）。
