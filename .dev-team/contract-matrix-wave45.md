# wave-45 契约矩阵（B 级 · 可观测性语义收敛）

> Protocol Agent 冻结产物（契约/NFR 唯一编写方）。事实源：`docs/audit/observability-audit-2026-10-25.md` O1/O2（`:44-49`、`:74-77`）+ wave-44 审查遗留。
> 写域：本波我**只**写 `.dev-team/`；`src/**`、`crates/**`、`tests/**` 归 Executor/Test Agent。
> **no new stubs**：禁止 `unimplemented!`/`todo!`/空函数体占位、禁止跳过用例与吞异常放行。
> 红相纪律：Rust 集成测试只可引用 **HEAD 已存在**的公有 API；未实现时必须因**断言失败**变红，**编译失败视为红相不合规**（故冻结断言只断言 wire 字面量，不引用待新增符号）。

## 0. 事实锚点（path:line）

| # | 事实 | 锚点 |
| :-- | :-- | :-- |
| F1 | `Resource::builder()` 仅装配 `service.name` | `crates/pony-agent-core/src/agent/otel/mod.rs:480-482` |
| F2 | `OtelConfig` 4 公共字段 + **7 处**字面构造点（改结构会破坏） | `.../otel/mod.rs:221-233`、`:2285`、`:2363`、`:2508`、`:2594`、`:2633`、`:2739`、`:2797` |
| F3 | 进程 env 白名单仅 4 项；有空值过滤工具 | `.../otel/mod.rs:419-423`、`:1582-1587` |
| F4 | 根 span 仅 `phase=="failed"` 置 ERROR，message=phase | `.../otel/mod.rs:1218-1226` |
| F5 | 工具失败谓词（failed/error/error.is_some） | `.../otel/mod.rs:1465-1479`（谓词 :1474） |
| F6 | 治理 block 已在子 span 置 `quota_blocked` | `.../otel/mod.rs:1448-1450` |
| F7 | 活动 turn：task-id 表 + 线程本地栈；成本聚合范式 | `.../otel/mod.rs:1657-1664`、`:1673-1687`、`:1802-1840` |
| F8 | 属性常量区（`ATTR_AGENT_TURN_OUTCOME` 就近插入处） | `.../otel/mod.rs:106`、`:115-121` |
| F9 | local-cache 裸 await（host-read 分支已用 withTimeout） | `src/stores/runtime.ts:2676-2691`、`:2693-2697` |
| F10 | `withTimeout`：超时 reject、结算清 timer | `src/lib/runtime/utils.ts:3-17` |
| F11 | 水合基线 15000ms；watchdog 3500ms 只解锁不取消 await | `src/lib/runtime/constants.ts:15`、`src/stores/runtime.ts:2639-2645` |
| F12 | local-cache 触发：`persisted.checkpoint≠null ∧ phase≠idle` | `src/lib/runtime/sessions.ts:665-676` |
| F13 | 缓存视图构建 / `mergeRuntimeViews(cached,null)===cached` | `src/lib/runtime/sessions.ts:400`、`:610-616` |
| F14 | 持久化 payload 含 `phase`/`checkpoint`（local-cache 生产可达） | `src/stores/runtime.ts:1067`、`:1084` |
| F15 | 版本双轨：core crate `0.1.128` / tauri `0.1.155`（`CARGO_PKG_VERSION` 取 core） | `crates/pony-agent-core/Cargo.toml:3`、`.version.json`、`scripts/check-version-sync.ps1:47-68` |
| F16 | 宿主装配点 `from_settings ⇒ init` | `src-tauri/src/lib.rs:1295-1296` |

[UNVERIFIED] collector/网关是否保留 resource 属性（审计已见 edge 重写 `service.name`）——repo 内无 collector 配置，无法离线断言 `service.version` 不被改写；[UNVERIFIED] 生产态 `deployment.environment` 只能由 `cfg!(debug_assertions)` 推导，preprod/staging 不可区分（结构性，见 Non-Goals）。

## 1. PA-153 · OTel resource 归因

| 缺陷锚点 | 黑盒期望（wire 级） | 可测接缝 | 冻结断言 |
| :-- | :-- | :-- | :-- |
| F1/F2：resource 只有 `service.name`，无版本/环境归因 | 每条导出 ResourceSpans 同时含 `service.name`、`service.version`、`deployment.environment` 且各出现一次；`service.version` 默认 = core crate 版本（本 checkout `0.1.128`） | 回环 OTLP sink（复用 `tests/otel_trace_migration_test.rs:190 spawn_otlp_sink` 范式）→ `init` + 1 turn + `force_flush` → 断言 body 的 ASCII 子串 | **PA153-T1** |
| F3/F15：无 env 覆盖面；`OtelConfig` 公共结构不可改 | `PONY_OTEL_SERVICE_VERSION`/`PONY_OTEL_ENVIRONMENT` trim 后非空即覆盖默认；空白串等同未设置；`OtelConfig` 4 字段与 7 处构造点零改动 | 同 sink；序列化用例内 `set_var` 两个哨兵值 → init → flush；再置空白串 → init → flush；退出恢复 env | **PA153-T2** |
| F11 类比：默认环境需真实区分 | debug 构建默认 `development`；release 构建默认 `production` | T1 在 debug 测试构建中断言 `development`；`production` 分支由实现期 in-crate 纯函数单测（入参布尔）覆盖，冻结集成测试不覆盖 release | PA153-T1（debug）/ 实现期单测（release） |

- T1 必含非空洞锚：body 同时含 `service.name` 与 `env!("CARGO_PKG_VERSION")`，否则「0 命中」无意义。
- 风险（已验证，非本波可修）：默认版本取 core crate（`0.1.128`），与发布版本（tauri `0.1.155`）不同轨；发布对齐只能靠 `PONY_OTEL_SERVICE_VERSION` 覆盖（宿主未设置，见 F16），列为 follow-up。

## 2. PA-154 · `agent.turn` 根 span 错误承接

| 缺陷锚点 | 黑盒期望（wire 级） | 可测接缝 | 冻结断言 |
| :-- | :-- | :-- | :-- |
| F4/F5：工具失败时根 span 仍 `status={}`，告警按根状态全漏 | 任一 `tool.call` 失败（谓词同 F5）且 `phase≠failed` ⇒ 根 span `status=ERROR` 且 `status.message` 以 ASCII **`turn_error: `** 开头、紧随 `tool_failed`（如 `turn_error: tool_failed: exit status 1`） | `begin_turn_trace` + `record_tool_call_span(status="failed", error=Some("exit status 1"))` + `set_phase("completed")` + drop + `force_flush` → body 含 `turn_error: ` 与 `tool_failed`；**子 span 文案不得带 `turn_error`**（唯一性即鉴别力） | **PA154-T1** |
| F4/F6：治理 block 同样不承接 | `record_verdict(...,"block",...)` 且 `phase≠failed` ⇒ 根 span ERROR，`status.message` 以 `turn_error: ` 开头、紧随 `quota_blocked`；注意 HEAD 的 governance 子 span 已写 `quota_blocked`（无鉴别力），鉴别力只能由 `turn_error: ` 提供 | 同 sink：`begin_turn_trace` + `start_governance_decide_span().record_verdict("session","block",Some("hard_limit_reached"),"v1",0)` + `set_phase("completed")` + drop + flush → body 含 **`turn_error: `** 且含 `quota_blocked` | **PA154-T2** |
| F4：无「本轮是否有错」的根级属性 | 有错 ⇒ 根 span 含 `agent.turn.outcome` 且**值必须为 `error`**（常量 `ATTR_AGENT_TURN_OUTCOME`）；全成功 ⇒ **不输出该键** | 失败用例同 T1/T2 断言 outcome 键**与值 `error`**；全成功用例（tool status="success"、无 block、phase=`completed`）断言该键、`turn_error: `、`tool_failed`/`quota_blocked` 均**不出现**，且必含 `agent.turn`+`agent.turn.phase`+`completed`（非空洞） | **PA154-T3** |

- **优先级（冻结）**：`phase=="failed"` ⇒ 既有 `status.message="failed"`（**不写** `turn_error`），但仍输出 `outcome="error"`；`phase` 完成且有子错误 ⇒ `turn_error: <tool_failed|quota_blocked>…` + `outcome="error"`；全成功 ⇒ 两者皆无。
- 语义边界：phase/cost 语义零变更（phase 仍输出真实值；`agent.turn.cost.usd` 仍按 Wave-5 K4 规则）；聚合按活动 turn 登记（task-id 表 / 线程本地栈，与成本聚合同源），无活动 turn 时 no-op，守卫 Drop 后清零（跨 turn 不污染）；双错误并存取先记录者（本波不冻结组合）。
- 可观测代理：不做 protobuf 结构解析，以 `status.message` 的 `turn_error: ` 前缀作为根级 ERROR 的可判代理（ASCII 子串）；零新增：无网络/IO/后台线程、不新增 span、不改门禁默认值。

## 3. PA-155 · local-cache 启动分支有界超时

| 缺陷锚点 | 黑盒期望 | 可测接缝 | 冻结断言 |
| :-- | :-- | :-- | :-- |
| F9/F10：local-cache 宿主读裸 await ⇒ IPC 挂起时 `initializeSessions()` 永不 settle，缓存会话打不开 | 宿主读必须 `withTimeout(hostRead, HYDRATION_TIMEOUT_MS, "加载对话")`；超时被 catch 成 `null`（不得冒泡为 `sessionError`），`mergeRuntimeViews(cached,null)` 原样保留 cached 视图并正常进入会话（`sessionId`=preferred、cached 消息可见、`phase` **不得为** `error`/`failed`/`cancelled`、`sessionOperation===null`） | vitest + fake timers：localStorage 写 `{phase:"calling_tool", checkpoint:createCheckpoint(...), messages:[cached…]}`（F12/F14）；`list_sessions` 返回同 id；`load_session_runtime_view` 返回永不 settle 的 Promise；推进 `HYDRATION_TIMEOUT_MS+1000` | **PA155-T1** |
| F9：迟到 resolve 可能迟到写；重试风暴 | 迟到 resolve 不得改写 `sessionId`/`messages`/`phase`；宿主读恰好 1 次（无重试）；timer 结算即清 | 同上，宿主读在 `15000+5000ms` 才 resolve 且返回**不同** host 文案 → 结算后断言 cached 文案在、host 文案不在，再 flush timers 逐项比对结算态无变化、调用次数仍 1 | **PA155-T2** |

- 常量漂移锁：`expect(HYDRATION_TIMEOUT_MS).toBe(15000)`；`settle_deadline_ms = HYDRATION_TIMEOUT_MS + 1000 = 16000`（只断「有界收敛」，不断壁钟）。
- 零改动边界：host-read 分支（`:2693-2697`）与 3500ms watchdog（`:2639-2645`）行为不变；host 读成功路径仍优先 host（merge 语义不变）。

## 4. Non-Goals（本波不做）

1. 不改 `OtelConfig` 公共结构、不改 settings 落盘结构、不新增 OTel 配置项（新增字段禁止）。
2. 不做 release 构建的 `production` 端到端断言（只用实现期纯函数单测覆盖）；不做 staging/preprod 环境语义。
3. 不做 `service.version` 与发布版本（tauri `0.1.155`）的自动对齐，不改宿主装配点（F16）。
4. 不改 phase / cost / 采样 / 导出队列语义；不为 `agent.turn.outcome` 之外的失败引入新属性。
5. 不改 host-read 分支、watchdog 超时值、`mergeRuntimeViews` 合并策略；不新增 retry。
6. 不动 `docs/` 与 `docs/observability.yaml`（本波无新增对外接口/网络出口/后台任务；声明归属由 Lead 判定）。
7. 不修 O3/O4/O5 等审计其它优化点、不触碰 collector/网关侧 resource 保留策略。

## 5. NFR 映射（详见 `.dev-team/nfr-baseline-wave45.json`）

| NFR 键 | 取值/判据 | 覆盖 |
| :-- | :-- | :-- |
| `wave45_observability_semantics.pa_153_resource_attribution` | resource 三键齐全；默认 `CARGO_PKG_VERSION`/`development`\|`production`；两个 env 覆盖；空白=未设；OtelConfig 不变；wire 子串 `service.version`/`deployment.environment`/`development` | PA-153 |
| `...pa_154_turn_error_aggregation` | `root_error_message_prefix="turn_error: "`（后接 `tool_failed` 或 `quota_blocked`）；`outcome_value_on_error="error"`（成功不输出该键）；phase=failed 优先（message=`failed`，不写 `turn_error`）；phase/cost 不变；聚合有界、Drop 清零；零新 IO/零 stub | PA-154 |
| `...pa_155_local_cache_timeout` | `hydration_timeout_ms=15000`（pin）、`settle_deadline_ms=16000`、cached 视图保留、迟到无写、host 读 1 次、watchdog=3500 不变 | PA-155 |
| `external_call_timeout_ms=3000` | 继承不变；PA-155 为本地 IPC 水合，`applicable=false`（结构性理由见 JSON） | 全波 |
| `logging{format=json,trace_id_required}` | 无新增结构化日志面；`debugLog` 属开发者诊断通道，不纳入 | 全波 |
| `resource_release` | `withTimeout` timer 结算即清 + watchdog `finally` 清理 + provider 退出 flush/shutdown 不变 | 全波 |

## 6. 冻结测试文件（Test Agent 落笔，锚定后只读）

- Rust：`crates/pony-agent-core/tests/otel_resource_turn_status_test.rs`（已落盘；持全局串行锁，范式对齐 `otel_trace_migration_test.rs`）。
- 前端：`tests/acceptance/wave45/pa155-local-cache-timeout.spec.ts`（PA155-T1/T2，与本节语义一致）。
- 断言 id 冻结：`PA153-T1/T2`、`PA154-T1/T2/T3`、`PA155-T1/T2`；Rust 冻结测试须断言 `turn_error: ` **唯一性**（子 span 不带）与 `agent.turn.outcome` **键值 = `error`**（非仅键存在）；放宽（删非空洞锚、改 deadline、改前缀字面量）须 Lead 批准。
- 覆盖差异（需 Lead 裁定）：已落 Rust 文件的 id 映射与本矩阵不同——缺「治理 block ⇒ `quota_blocked`」（PA-154 必测项）与两个 env 覆盖（PA-153 必测项）；应补测或由 Lead 重映射 id，不得删矩阵要求。
